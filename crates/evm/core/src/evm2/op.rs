//! OP Stack on evm2, using `op-evm2`.

use super::{
    DepositTxExt, EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxEnv, FoundryTxResultExt, build_eth_tx,
};
use crate::evm::OpEvmNetwork;
use alloy_consensus::transaction::Recovered;
use evm2::{
    TxResult, Version, ethereum::TxEnvelope, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use op_alloy_network::Optimism;
use op_evm2::{
    OpBlockEnvExt, OpEvmExt, OpEvmFactory, OpEvmTypes, OpPrecompiles, OpSpecId, OpTxEnvelope,
    OpTxResultExt, OpTypedTx, TxDeposit, op_tx_registry, op_version,
};

impl FoundryBlockExt for OpBlockEnvExt {}

impl FoundryEvmExt<DepositTxExt> for OpEvmExt {
    fn clear_transaction_fee_cache(&mut self) {
        self.clear_tx_l1_cost();
    }
}

impl FoundryTxResultExt for OpTxResultExt {
    fn is_failed_deposit(&self) -> bool {
        self.failed_deposit
    }
}

impl FoundryEvmNetwork for OpEvmNetwork {
    type Network = Optimism;
    type EvmFactory = OpEvmFactory;
}

impl FoundryEvmFactory for OpEvmFactory {
    type Types = OpEvmTypes;
    type TxExt = DepositTxExt;

    fn version(&self, spec: OpSpecId, chain_id: u64) -> Version {
        op_version(spec, chain_id)
    }

    fn tx_registry(&self, spec: OpSpecId) -> TxRegistry<OpEvmTypes, TxResult<OpEvmTypes>> {
        op_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<OpEvmTypes>,
        _ext: &OpEvmExt,
    ) -> Box<dyn PrecompileProvider<OpEvmTypes>> {
        Box::new(OpPrecompiles::new(env.spec))
    }

    fn build_tx(&self, tx: &FoundryTxEnv<DepositTxExt>) -> eyre::Result<Recovered<OpTxEnvelope>> {
        let envelope = if tx.is_deposit() {
            OpTxEnvelope::deposit(TxDeposit {
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
            let typed = match build_eth_tx(tx)? {
                TxEnvelope::Legacy(tx) => OpTypedTx::Legacy(tx),
                TxEnvelope::Eip2930(tx) => OpTypedTx::Eip2930(tx),
                TxEnvelope::Eip1559(tx) => OpTypedTx::Eip1559(tx),
                TxEnvelope::Eip7702(tx) => OpTypedTx::Eip7702(tx),
                TxEnvelope::Eip4844(_) => {
                    eyre::bail!("OP Stack does not support blob transactions")
                }
            };
            OpTxEnvelope::new(typed, Some(tx.ext.enveloped_tx.clone().unwrap_or_default()))
        };
        Ok(Recovered::new_unchecked(envelope, tx.caller))
    }
}
