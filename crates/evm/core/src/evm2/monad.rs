//! Monad on evm2, using `monad-evm2`.

use super::{
    EvmEnv, FoundryBlockExt, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork,
    FoundryTxResultExt, TxPayload, eth_tx,
};
use crate::evm::MonadEvmNetwork;
use alloy_consensus::transaction::Recovered;
use alloy_network::Ethereum;
use alloy_primitives::Address;
use evm2::{
    TxResult, Version, ethereum::TxEnvelope, evm::precompile::PrecompileProvider,
    registry::TxRegistry,
};
use monad_evm2::{
    MonadBlockExt, MonadChainContext, MonadEvmExt, MonadEvmFactory, MonadEvmTypes, MonadHardfork,
    MonadPrecompiles, MonadTxResultExt, monad_tx_registry, monad_version,
};

impl FoundryBlockExt for MonadBlockExt {}

impl FoundryEvmExt<TxEnvelope> for MonadEvmExt {
    fn for_transaction(&mut self, tx: &Recovered<TxEnvelope>) {
        // A synthetic transaction is alone in its block: its sender and EIP-7702 authorities are
        // the only ones the reserve-balance rules may see.
        let authorities = tx
            .as_eip7702()
            .map(|tx| tx.authorization_list.iter().filter_map(|auth| auth.authority()).collect())
            .unwrap_or_default();
        self.chain = MonadChainContext {
            current_block_senders: vec![tx.signer()],
            current_block_authorities: vec![authorities],
            current_tx_index: 0,
            ..Default::default()
        };
    }
}

impl FoundryTxResultExt for MonadTxResultExt {}

impl FoundryEvmNetwork for MonadEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = MonadEvmFactory;

    fn evm_tx(
        envelope: alloy_consensus::TxEnvelope,
        sender: Address,
        _payload: TxPayload,
    ) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(eth_tx(envelope), sender))
    }
}

impl FoundryEvmFactory for MonadEvmFactory {
    type Types = MonadEvmTypes;

    fn version(&self, spec: MonadHardfork, chain_id: u64) -> Version {
        monad_version(spec, chain_id)
    }

    fn tx_registry(
        &self,
        spec: MonadHardfork,
    ) -> TxRegistry<MonadEvmTypes, TxResult<MonadEvmTypes>> {
        monad_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<MonadEvmTypes>,
        _ext: &MonadEvmExt,
    ) -> Box<dyn PrecompileProvider<MonadEvmTypes>> {
        Box::new(MonadPrecompiles::new(env.spec))
    }
}
