//! Implementations of [`Config`](spec::Group::Config) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;

impl Cheatcode for configExistsCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { key } = self;
        let exists = state.config_storage.contains_key(key);
        Ok(exists.abi_encode())
    }
}

impl Cheatcode for getConfigCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { key } = self;
        match state.config_storage.get(key) {
            Some(value) => Ok(value.abi_encode()),
            None => Err(fmt_err!("NotInitialized: config key '{}' does not exist", key)),
        }
    }
}

impl Cheatcode for setConfigCall {
    fn apply(&self, state: &mut Cheatcodes) -> Result {
        let Self { key, value } = self;
        state.config_storage.insert(key.clone(), value.clone());
        Ok(Default::default())
    }
}
