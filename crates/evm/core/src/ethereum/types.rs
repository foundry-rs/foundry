//! EVM type family and opcode behavior for Foundry execution.

use alloy_primitives::{Address, U256};
use evm2::{
    BaseEvmConfig, Evm, EvmConfig, EvmConfigSelector, EvmTypesHost, ExecutionConfig, OpcodeConfig,
    SpecId,
    ethereum::TxEnvelope,
    interpreter::{Word, op},
};
use evm2_macros::instruction;

/// Context that Foundry cheatcodes may change during a transaction.
#[derive(Clone, Copy, Debug, Default)]
pub struct FoundryContext {
    /// Original sender of the transaction that owns the current execution.
    pub transaction_origin: Option<Address>,
    /// Overrides the origin reported by the ORIGIN opcode while a prank or broadcast is active.
    pub origin_override: Option<Address>,
    /// Preserves the configured base fee when synthetic transaction accounting uses zero base fee.
    pub basefee_override: Option<U256>,
    /// Preserves the configured gas price when synthetic transaction accounting uses zero price.
    pub gas_price_override: Option<U256>,
}

/// Ethereum EVM types with Foundry's live execution context.
#[derive(Clone, Copy, Debug)]
pub struct FoundryEvmTypes;

impl EvmTypesHost for FoundryEvmTypes {
    type ConfigSelector = FoundryConfigSelector;
    type SpecId = SpecId;
    type Tx = TxEnvelope;
    type EvmExt = FoundryContext;
    type MessageExt = ();
    type MessageResultExt = ();
    type TxEnvExt = ();
    type TxResultExt = ();
    type BlockEnvExt = ();
    type Host<'a> = Evm<'a, Self>;
}

/// Selects the Foundry opcode table for an Ethereum specification.
#[doc(hidden)]
pub struct FoundryConfigSelector;

impl EvmConfigSelector<FoundryEvmTypes> for FoundryConfigSelector {
    type Config<const BASE_SPEC_ID: u32, const CUSTOM_SPEC_ID: u32> = FoundryConfig<BASE_SPEC_ID>;

    fn execution_config(spec_id: SpecId) -> ExecutionConfig<FoundryEvmTypes> {
        evm2::spec_to_generic!(spec_id, |SPEC| ExecutionConfig::for_config::<FoundryConfig<SPEC>>())
    }
}

/// Ethereum opcode configuration with Foundry's live-origin behavior.
#[doc(hidden)]
pub struct FoundryConfig<const BASE_SPEC_ID: u32>;

impl<const BASE_SPEC_ID: u32> EvmConfig<FoundryEvmTypes> for FoundryConfig<BASE_SPEC_ID> {
    const BASE_SPEC_ID: SpecId =
        SpecId::try_from_u32(BASE_SPEC_ID).expect("invalid Foundry specification");
    const OPCODE_CONFIG: &'static OpcodeConfig<FoundryEvmTypes> = &{
        let mut config = OpcodeConfig::base::<BaseEvmConfig<BASE_SPEC_ID>>();
        config.set_instruction::<foundry_origin>(op::ORIGIN, 2);
        config.set_instruction::<foundry_gasprice>(op::GASPRICE, 2);
        if BASE_SPEC_ID >= SpecId::LONDON as u32 {
            config.set_instruction::<foundry_basefee>(op::BASEFEE, 2);
        }
        config
    };
}

#[instruction(EvmTypes = FoundryEvmTypes)]
fn foundry_origin(cx: _) -> out {
    let origin = cx.state.tx().origin;
    let origin = cx.state.host().ext().origin_override.unwrap_or(origin);
    *out = Word::from_be_slice(origin.as_slice());
}

#[instruction(EvmTypes = FoundryEvmTypes)]
fn foundry_basefee(cx: _) -> out {
    let host = cx.state.host();
    *out = host.ext().basefee_override.unwrap_or(host.block().basefee);
}

#[instruction(EvmTypes = FoundryEvmTypes)]
fn foundry_gasprice(cx: _) -> out {
    let gas_price = cx.state.tx().gas_price;
    *out = cx.state.host().ext().gas_price_override.unwrap_or(gas_price);
}
