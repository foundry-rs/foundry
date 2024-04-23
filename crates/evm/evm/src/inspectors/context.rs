use alloy_primitives::U256;
use revm::{
    interpreter::{CallInputs, CallOutcome},
    Database, EvmContext, Inspector,
};
use serde::{Deserialize, Serialize};

/// An execution context
#[derive(Clone, Default, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct Context {
    /// The block number of the context.
    pub block_number: U256,
}

/// An inspector that collects EVM context during execution.
#[derive(Clone, Debug, Default)]
pub struct ContextCollector {
    /// The collected execution contexts.
    pub contexts: Vec<Context>,
}

impl<DB: Database> Inspector<DB> for ContextCollector {
    fn call(&mut self, ecx: &mut EvmContext<DB>, _call: &mut CallInputs) -> Option<CallOutcome> {
        let block_number = ecx.inner.env.block.number;

        // Skip if the previous context is the same
        if let Some(Context { block_number: prev_block_number }) = self.contexts.last() {
            if *prev_block_number == block_number {
                return None;
            }
        }

        // Push the new context
        self.contexts.push(Context { block_number });

        None
    }
}
