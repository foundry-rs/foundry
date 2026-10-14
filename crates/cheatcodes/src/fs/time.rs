//! Context-free host time cheatcode.

use crate::{Cheatcode, Result, Vm::unixTimeCall};
use alloy_sol_types::SolValue;
use std::time::{SystemTime, UNIX_EPOCH};

impl Cheatcode for unixTimeCall {
    fn apply_context_free(&self) -> Result {
        let Self {} = self;
        let difference = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| fmt_err!("failed getting Unix timestamp: {e}"))?;
        Ok(difference.as_millis().abi_encode())
    }
}
