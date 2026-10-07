//! The engine validator: reth's Ethereum validator, except that payload attributes may carry a
//! timestamp at or below the parent's, as anvil mines blocks with the same or an earlier
//! timestamp when `evm_mine` or `evm_setTime` ask for one, and that a payload of a block before
//! London converts to a header without a base fee, which the engine API cannot express.

use alloy_rpc_types_engine::{ExecutionData, PayloadAttributes, PayloadError};
use reth_ethereum::{
    Block, EthPrimitives,
    chainspec::{EthChainSpec, EthereumHardforks, Hardforks},
    node::{
        EthereumEngineValidator,
        api::{
            EngineApiMessageVersion, EngineApiValidator, EngineObjectValidationError, EngineTypes,
            FullNodeComponents, InvalidPayloadAttributesError, NewPayloadError, NodeTypes,
            PayloadOrAttributes, PayloadTypes, PayloadValidator,
        },
        builder::{AddOnsContext, rpc::PayloadValidatorBuilder},
    },
    primitives::{Block as BlockTrait, SealedBlock},
};
use std::sync::Arc;

/// Reth's Ethereum engine validator without the timestamp check on payload attributes, and with
/// payloads of blocks before London.
#[derive(Debug, Clone)]
pub struct AnvilEngineValidator<ChainSpec> {
    inner: EthereumEngineValidator<ChainSpec>,
    chain_spec: Arc<ChainSpec>,
}

impl<ChainSpec, Types> PayloadValidator<Types> for AnvilEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<ExecutionData = ExecutionData>,
{
    type Block = Block;

    fn convert_payload_to_block(
        &self,
        payload: ExecutionData,
    ) -> Result<SealedBlock<Self::Block>, NewPayloadError> {
        if self.chain_spec.is_london_active_at_block(payload.payload.block_number()) {
            return PayloadValidator::<Types>::convert_payload_to_block(&self.inner, payload);
        }
        // The engine API always carries a base fee; a block before London has none.
        let ExecutionData { payload, sidecar } = payload;
        let expected = payload.block_hash();
        let mut block: Block = payload.try_into_block_with_sidecar(&sidecar)?;
        block.header.base_fee_per_gas = None;
        let block = BlockTrait::seal_slow(block);
        if block.hash() != expected {
            return Err(
                PayloadError::BlockHash { execution: block.hash(), consensus: expected }.into()
            );
        }
        Ok(block)
    }

    fn validate_payload_attributes_against_header(
        &self,
        _attr: &Types::PayloadAttributes,
        _header: &<Self::Block as BlockTrait>::Header,
    ) -> Result<(), InvalidPayloadAttributesError> {
        Ok(())
    }
}

impl<ChainSpec, Types> EngineApiValidator<Types> for AnvilEngineValidator<ChainSpec>
where
    ChainSpec: EthChainSpec + EthereumHardforks + 'static,
    Types: PayloadTypes<PayloadAttributes = PayloadAttributes, ExecutionData = ExecutionData>,
{
    fn validate_version_specific_fields(
        &self,
        version: EngineApiMessageVersion,
        payload_or_attrs: PayloadOrAttributes<'_, Types::ExecutionData, PayloadAttributes>,
    ) -> Result<(), EngineObjectValidationError> {
        EngineApiValidator::<Types>::validate_version_specific_fields(
            &self.inner,
            version,
            payload_or_attrs,
        )
    }

    fn ensure_well_formed_attributes(
        &self,
        version: EngineApiMessageVersion,
        attributes: &PayloadAttributes,
    ) -> Result<(), EngineObjectValidationError> {
        EngineApiValidator::<Types>::ensure_well_formed_attributes(&self.inner, version, attributes)
    }
}

/// Builds the [`AnvilEngineValidator`].
#[derive(Debug, Default, Clone)]
pub struct AnvilEngineValidatorBuilder;

impl<Node, Types> PayloadValidatorBuilder<Node> for AnvilEngineValidatorBuilder
where
    Types: NodeTypes<
            ChainSpec: Hardforks + EthereumHardforks + Clone + 'static,
            Payload: EngineTypes<ExecutionData = ExecutionData>
                         + PayloadTypes<PayloadAttributes = PayloadAttributes>,
            Primitives = EthPrimitives,
        >,
    Node: FullNodeComponents<Types = Types>,
{
    type Validator = AnvilEngineValidator<Types::ChainSpec>;

    async fn build(self, ctx: &AddOnsContext<'_, Node>) -> eyre::Result<Self::Validator> {
        let chain_spec = ctx.config.chain.clone();
        Ok(AnvilEngineValidator {
            inner: EthereumEngineValidator::new(chain_spec.clone()),
            chain_spec,
        })
    }
}
