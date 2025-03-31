pub use crate::ic::*;
use crate::{
    evm::{FoundryEvm, FoundryEvmCtx},
    InspectorExt,
};

use alloy_primitives::map::foldhash::HashMap;
use revm::{
    context::{
        result::{EVMError, HaltReason},
        ContextTr, Transaction,
    },
    handler::{
        instructions::{EthInstructions, InstructionProvider},
        EthFrame, EvmTr, Handler, ItemOrResult,
    },
    interpreter::interpreter::EthInterpreter,
    Database,
};

/// A list of features that can be enabled or disabled in the [`FoundryHandler`].
/// This is used to conditionally override certain execution paths in the EVM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Features {
    Create2Handler,
}

/// A [`Handler`] registry for the Foundry EVM.
/// This is a wrapper around the EVM that allows us to conditionally override certain
/// execution paths based on the enabled features.
pub struct FoundryHandler<'db, I: InspectorExt> {
    /// A map of enabled features.
    pub enabled: HashMap<Features, bool>,
    /// The inner EVM instance.
    pub inner: FoundryEvm<'db, I>,
}

impl<'db, I: InspectorExt> FoundryHandler<'db, I> {
    /// Creates a new [`FoundryHandler`] with the given context and inspector.
    pub fn new(ctx: FoundryEvmCtx<'db>, inspector: I) -> Self {
        FoundryHandler { inner: FoundryEvm::new(ctx, inspector), enabled: HashMap::default() }
    }

    /// Set the enabled state of a handler feature.
    pub fn set_enabled(&mut self, name: Features, enabled: bool) {
        self.enabled.insert(name, enabled);
    }

    /// Whether a feature is enabled or not.
    pub fn is_enabled(&self, name: Features) -> bool {
        self.enabled.get(&name).copied().unwrap_or(false)
    }
}

impl<'db, I> Handler for FoundryHandler<'db, I>
where
    I: InspectorExt,
{
    type Evm = FoundryEvm<'db, I>;
    type Error = EVMError<<<FoundryEvmCtx<'db> as ContextTr>::Db as Database>::Error>;
    type Frame = EthFrame<
    Self::Evm,
    Self::Error,
    <EthInstructions<
        EthInterpreter,
        FoundryEvmCtx<'db>,
    > as InstructionProvider>::InterpreterTypes,
>;
    type HaltReason = HaltReason;

    fn execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &revm::interpreter::InitialAndFloorGas,
    ) -> Result<revm::handler::FrameResult, Self::Error> {
        let gas_limit = evm.ctx().tx().gas_limit() - init_and_floor_gas.initial_gas;

        // Create first frame action
        let first_frame_input = self.first_frame_input(evm, gas_limit)?;
        let first_frame = self.first_frame_init(evm, first_frame_input)?;
        let mut frame_result = match first_frame {
            ItemOrResult::Item(frame) => self.run_exec_loop(evm, frame)?,
            ItemOrResult::Result(result) => result,
        };

        self.last_frame_result(evm, &mut frame_result)?;
        Ok(frame_result)
    }
}
