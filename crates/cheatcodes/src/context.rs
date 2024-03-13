//! Implementations of [`Context`](crate::Group::Context) cheatcodes.

use crate::{Cheatcode, Cheatcodes, Result, Vm::*};
use alloy_sol_types::SolValue;
use once_cell::sync::OnceCell;
use std::mem::discriminant;

/// Stores the forge execution context for the duration of the program.
static FORGE_CONTEXT: OnceCell<ForgeContext> = OnceCell::new();

/// Possible forge execution contexts.
pub enum ForgeContext {
    /// `forge test` command execution context.
    TestStandard,
    /// `forge coverage` command execution context.
    TestCoverage,
    /// `forge snapshot` command execution context.
    TestSnapshot,
    /// `forge script` command execution context.
    ScriptDryRun,
    /// `forge script --broadcast` command execution context.
    ScriptBroadcast,
    /// `forge script --resume` command execution context.
    ScriptResume,
    /// Unknown `forge` command execution context.
    Unknown,
}

impl ForgeContext {
    /// Set `forge` command current execution context for the duration of the program.
    /// Execution context is immutable, subsequent calls of this function won't change the context.
    pub fn set_execution_context(context: ForgeContext) {
        let _ = FORGE_CONTEXT.set(context);
    }
}

impl Cheatcode for isTestContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok((is_forge_context(ForgeContext::TestStandard) ||
            is_forge_context(ForgeContext::TestCoverage) ||
            is_forge_context(ForgeContext::TestSnapshot))
        .abi_encode())
    }
}

impl Cheatcode for isTestCoverageContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::TestCoverage).abi_encode())
    }
}

impl Cheatcode for isTestSnapshotContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::TestSnapshot).abi_encode())
    }
}

impl Cheatcode for isTestStandardContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::TestStandard).abi_encode())
    }
}

impl Cheatcode for isScriptContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok((is_forge_context(ForgeContext::ScriptDryRun) ||
            is_forge_context(ForgeContext::ScriptBroadcast) ||
            is_forge_context(ForgeContext::ScriptResume))
        .abi_encode())
    }
}

impl Cheatcode for isScriptBroadcastContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::ScriptBroadcast).abi_encode())
    }
}

impl Cheatcode for isScriptDryRunContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::ScriptDryRun).abi_encode())
    }
}

impl Cheatcode for isScriptResumeContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::ScriptResume).abi_encode())
    }
}

fn is_forge_context(context: ForgeContext) -> bool {
    discriminant(&context) == discriminant(FORGE_CONTEXT.get().unwrap())
}
