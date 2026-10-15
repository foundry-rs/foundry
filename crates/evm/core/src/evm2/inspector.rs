//! Foundry inspector extensions.

use crate::constants::DEFAULT_CREATE2_DEPLOYER;
use alloy_primitives::Address;
use evm2::{EvmTypesHost, Inspector, NoopInspector, interpreter::Message};
use foundry_evm_networks::NetworkConfigs;

/// Successor of [`crate::FoundryInspectorExt`] over evm2's [`Inspector`].
///
/// The Foundry hooks are unchanged apart from receiving evm2 [`Message`]s instead of REVM
/// `CreateInputs`.
pub trait FoundryInspectorExt<T: EvmTypesHost>: Inspector<T> {
    /// Determines whether the `DEFAULT_CREATE2_DEPLOYER` should be used for a CREATE2 message.
    ///
    /// If this returns true, the CREATE2 message is replaced with a call to the CREATE2 factory.
    fn should_use_create2_factory(&mut self, _depth: usize, _message: &Message<T>) -> bool {
        false
    }

    /// Simulates `console.log` invocation.
    fn console_log(&mut self, _msg: &str) {}

    /// Returns configuration retained for Celo precompile support.
    fn get_networks(&self) -> NetworkConfigs {
        NetworkConfigs::default()
    }

    /// Returns the CREATE2 deployer address.
    fn create2_deployer(&self) -> Address {
        DEFAULT_CREATE2_DEPLOYER
    }
}

impl<T: EvmTypesHost> FoundryInspectorExt<T> for NoopInspector {}
