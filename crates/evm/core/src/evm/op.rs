//! OP Stack on evm2, using `op-evm2`.

use super::{
    EvmEnv, FoundryEvmExt, FoundryEvmFactory, FoundryEvmNetwork, FoundryTxResultExt, SendableExt,
    TxPayload,
};
use alloy_consensus::transaction::Recovered;
use alloy_eips::Encodable2718;
use alloy_network::AnyRpcTransaction;
use alloy_primitives::{Address, Bytes};
use evm2::{TxResult, Version, evm::precompile::PrecompileProvider, registry::TxRegistry};
use op_alloy_consensus::OpTxEnvelope as OpConsensusTxEnvelope;
use op_alloy_network::Optimism;
use op_evm2::{
    OpEvmExt, OpEvmFactory, OpEvmTypes, OpPrecompiles, OpSpecId, OpTxEnvelope, OpTxResultExt,
    OpTypedTx, TxDeposit, op_tx_registry, op_version,
};

/// OP Stack EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct OpEvmNetwork;

// SAFETY: `OpEvmExt` is `Send + Sync`, as asserted below, so clones share only thread-safe state.
unsafe impl SendableExt for OpEvmExt {}

const _: fn() = || {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<OpEvmExt>();
};

impl FoundryEvmExt<OpTxEnvelope> for OpEvmExt {
    fn replace_while_executing(&mut self, ext: Self) {
        // Only the block-level L1 info comes from `ext`; everything else, including the cached L1
        // cost, describes the running transaction, as when REVM kept this state in its context.
        let tx_l1_cost = self.l1_block_info.tx_l1_cost;
        self.l1_block_info = ext.l1_block_info;
        self.l1_block_info.tx_l1_cost = tx_l1_cost;
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

    fn evm_tx(
        envelope: OpConsensusTxEnvelope,
        sender: Address,
        payload: TxPayload,
    ) -> eyre::Result<Recovered<OpTxEnvelope>> {
        // The L1 data fee is priced over the signed encoding; simulations of unsigned requests
        // price it at zero, like `eth_call`.
        let enveloped = Some(if payload.is_simulated() {
            Bytes::new()
        } else {
            envelope.encoded_2718().into()
        });
        let typed = match envelope {
            OpConsensusTxEnvelope::Legacy(tx) => OpTypedTx::Legacy(tx.strip_signature()),
            OpConsensusTxEnvelope::Eip2930(tx) => OpTypedTx::Eip2930(tx.strip_signature()),
            OpConsensusTxEnvelope::Eip1559(tx) => OpTypedTx::Eip1559(tx.strip_signature()),
            OpConsensusTxEnvelope::Eip7702(tx) => OpTypedTx::Eip7702(
                evm2::ethereum::LazyTxEip7702::from_recovered_authorizations(tx.strip_signature()),
            ),
            OpConsensusTxEnvelope::Deposit(tx) => {
                let tx = tx.into_inner();
                return Ok(Recovered::new_unchecked(
                    OpTxEnvelope::deposit(TxDeposit {
                        source_hash: tx.source_hash,
                        from: tx.from,
                        to: tx.to,
                        mint: tx.mint,
                        value: tx.value,
                        gas_limit: tx.gas_limit,
                        is_system_transaction: tx.is_system_transaction,
                        input: tx.input,
                    }),
                    sender,
                ));
            }
            OpConsensusTxEnvelope::PostExec(_) => {
                eyre::bail!("OP post-execution transactions are not supported by op-evm2")
            }
        };
        Ok(Recovered::new_unchecked(OpTxEnvelope::new(typed, enveloped), sender))
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<OpConsensusTxEnvelope> {
        OpConsensusTxEnvelope::try_from(tx.clone())
            .map_err(|err| super::unknown_transaction_type(tx, "OpTransaction").wrap_err(err))
    }
}

impl FoundryEvmFactory for OpEvmFactory {
    type Types = OpEvmTypes;

    fn version(&self, spec: OpSpecId, chain_id: u64) -> Version {
        let mut version = op_version(spec, chain_id);
        // Like REVM, existing EIP-7702 delegations resolve on every spec; only authorization
        // processing is fork-gated, by the transaction registry.
        version.features.insert(evm2::EvmFeatures::EIP7702);
        version
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
}
