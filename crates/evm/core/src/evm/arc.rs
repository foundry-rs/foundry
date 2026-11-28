//! Arc on evm2, using `arc-evm2`: native USDC transfer rules and Arc precompiles.

use super::{
    EvmEnv, FoundryEvmFactory, FoundryEvmNetwork, FoundryExecutionHooks, FoundryTxConversion,
    TxPayload, eth_envelope_from_any_rpc, eth_tx, eth_tx_from_request, with_eip7702_delegations,
};
use crate::{constants::SYSTEM_PRECOMPILE_STUB, edgecov};
use alloy_consensus::transaction::Recovered;
use alloy_network::{AnyRpcTransaction, Ethereum};
use alloy_primitives::{Address, Bytes};
use alloy_rpc_types::TransactionRequest;
use arc_evm2::{
    ArcEvmFactory, ArcEvmTypes, ArcSpecId, arc_opcode_config, arc_precompiles, arc_tx_registry,
    arc_version,
};
use evm2::{
    Evm, EvmConfig, ExecutionConfig, OpcodeConfig, SpecId, TxResult, Version, bytecode::Bytecode,
    ethereum::TxEnvelope, evm::precompile::PrecompileProvider, registry::TxRegistry,
};

/// Arc EVM network.
#[derive(Clone, Copy, Debug, Default)]
pub struct ArcEvmNetwork;

impl FoundryEvmNetwork for ArcEvmNetwork {
    type Network = Ethereum;
    type EvmFactory = ArcEvmFactory;
}

impl FoundryExecutionHooks for ArcEvmTypes {}

impl FoundryTxConversion<Ethereum> for ArcEvmTypes {
    fn evm_tx(
        envelope: alloy_consensus::TxEnvelope,
        sender: Address,
        _payload: TxPayload,
    ) -> eyre::Result<Recovered<TxEnvelope>> {
        Ok(Recovered::new_unchecked(eth_tx(envelope), sender))
    }

    fn tx_from_request(request: TransactionRequest) -> eyre::Result<Recovered<TxEnvelope>> {
        eth_tx_from_request(request)
    }

    fn envelope_from_any_rpc(tx: &AnyRpcTransaction) -> eyre::Result<alloy_consensus::TxEnvelope> {
        eth_envelope_from_any_rpc(tx, "Arc")
    }
}

impl FoundryEvmFactory for ArcEvmFactory {
    type Types = ArcEvmTypes;

    fn version(&self, spec: ArcSpecId, chain_id: u64) -> Version {
        with_eip7702_delegations(arc_version(spec, chain_id))
    }

    fn execution_config(&self, spec: ArcSpecId, version: Version) -> ExecutionConfig<ArcEvmTypes> {
        evm2::spec_to_generic!(SpecId::from(spec), |BASE_SPEC_ID| ExecutionConfig::for_config::<
            InstrumentedArcConfig<BASE_SPEC_ID>,
        >()
        .with_version(version))
    }

    fn tx_registry(&self, spec: ArcSpecId) -> TxRegistry<ArcEvmTypes, TxResult<ArcEvmTypes>> {
        arc_tx_registry(spec)
    }

    fn precompiles(
        &self,
        env: &EvmEnv<ArcEvmTypes>,
        _ext: &(),
    ) -> Box<dyn PrecompileProvider<ArcEvmTypes>> {
        Box::new(arc_precompiles(env.spec))
    }

    fn initialize_evm(&self, evm: &mut Evm<'_, ArcEvmTypes>, _is_forked: bool) {
        // On Arc, the precompile accounts hold code `0xef` with nonce 1; existing state wins.
        let stub = Bytecode::new_legacy(Bytes::from_static(SYSTEM_PRECOMPILE_STUB));
        for address in arc_evm2::ARC_PRECOMPILE_ADDRESSES {
            if let Ok(mut account) = evm.state_mut().account(&address)
                && account.is_empty_code_hash()
            {
                account.set_code_slow(stub.clone());
                if account.nonce() == 0 {
                    account.set_nonce(1);
                }
            }
        }
        evm.state_mut().commit_transaction();
    }
}

/// Arc's opcode table of `BASE_SPEC_ID` with the [`edgecov`] instrumentation opcodes.
struct InstrumentedArcConfig<const BASE_SPEC_ID: u32>;

impl<const BASE_SPEC_ID: u32> EvmConfig<ArcEvmTypes> for InstrumentedArcConfig<BASE_SPEC_ID> {
    const BASE_SPEC_ID: SpecId =
        SpecId::try_from_u32(BASE_SPEC_ID).expect("invalid Arc base spec id");
    const OPCODE_CONFIG: &'static OpcodeConfig<ArcEvmTypes> =
        &edgecov::with_instrumentation(arc_opcode_config::<BASE_SPEC_ID>());
}
