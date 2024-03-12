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
    Test,
    /// `forge coverage` command execution context.
    Coverage,
    /// `forge snapshot` command execution context.
    Snapshot,
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
        Ok(is_forge_context(ForgeContext::Test).abi_encode())
    }
}

impl Cheatcode for isCoverageContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::Coverage).abi_encode())
    }
}

impl Cheatcode for isSnapshotContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::Snapshot).abi_encode())
    }
}

impl Cheatcode for isScriptDryRunContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::ScriptDryRun).abi_encode())
    }
}

impl Cheatcode for isScriptBroadcastContextCall {
    fn apply(&self, _state: &mut Cheatcodes) -> Result {
        Ok(is_forge_context(ForgeContext::ScriptBroadcast).abi_encode())
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
