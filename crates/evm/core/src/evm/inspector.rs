//! Foundry inspector extensions.

use crate::constants::DEFAULT_CREATE2_DEPLOYER;
use alloy_primitives::Address;
use evm2::{EvmTypesHost, Inspector, NoopInspector, interpreter::Message};

/// Foundry's hooks on top of evm2's [`Inspector`].
pub trait FoundryInspectorExt<T: EvmTypesHost>: Inspector<T> {
    /// Determines whether the `DEFAULT_CREATE2_DEPLOYER` should be used for a CREATE2 message.
    ///
    /// If this returns true, the CREATE2 message is replaced with a call to the CREATE2 factory.
    fn should_use_create2_factory(&mut self, _depth: usize, _message: &Message<T>) -> bool {
        false
    }

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, _msg: &str) {}

    /// Returns the CREATE2 deployer address.
    fn create2_deployer(&self) -> Address {
        DEFAULT_CREATE2_DEPLOYER
    }
}

impl<T: EvmTypesHost> FoundryInspectorExt<T> for NoopInspector {}
