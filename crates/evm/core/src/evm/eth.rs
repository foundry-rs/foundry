//! Ethereum on evm2.

use super::{
    EvmEnv, FoundryEvmFactory, FoundryEvmNetwork, TxPayload, eth_envelope_from_any_rpc, eth_tx,
};
use alloy_consensus::transaction::Recovered;
use alloy_network::{AnyRpcTransaction, Ethereum};
use alloy_primitives::Address;
use evm2::{
    BaseEvmTypes, Precompiles, SpecId, TxResult, Version, ethereum::TxEnvelope,
    evm::precompile::PrecompileProvider, registry::TxRegistry,
};

/// Ethereum evm2 factory over evm2's stock type family.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmFactory;

/// Ethereum EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct EthEvmNetwork;

impl FoundryEvmNetwork for EthEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = EthEvmFactory;

    fn evm_tx(
        envelope: alloy_consensus::TxEnvelope,
        sender: Address,
        _payload: TxPayload,
    ) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(eth_tx(envelope), sender))
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<alloy_consensus::TxEnvelope> {
        eth_envelope_from_any_rpc(tx, "TxEnv")
    }
}

impl FoundryEvmFactory for EthEvmFactory {
    type Types = BaseEvmTypes;

    fn version(&self, spec: SpecId, chain_id: u64) -> Version {
        let mut version = Version::new(spec);
        version.chain_id = chain_id;
        version
    }

    fn tx_registry(&self, spec: SpecId) -> TxRegistry<BaseEvmTypes, TxResult<BaseEvmTypes>> {
        evm2::ethereum::ethereum_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<BaseEvmTypes>,
        _ext: &(),
    ) -> Box<dyn PrecompileProvider<BaseEvmTypes>> {
        Box::new(Precompiles::base(env.spec))
    }
}
