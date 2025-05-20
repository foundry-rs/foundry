use alloy_primitives::{Address, U256};
use alloy_sol_types::SolValue;
use foundry_evm_core::{
    backend::DatabaseExt,
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
};
use revm::{
    interpreter::{
        opcode::{EXTCODESIZE, REVERT},
        CallInputs, CallOutcome, CallScheme, InstructionResult, Interpreter, InterpreterAction,
        InterpreterResult,
    },
    precompile::{PrecompileSpecId, Precompiles},
    primitives::SpecId,
    Database, EvmContext, Inspector,
};
use std::fmt;

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// Checks if the call scheme corresponds to any sort of delegate call
pub fn is_delegatecall(scheme: CallScheme) -> bool {
    matches!(scheme, CallScheme::DelegateCall | CallScheme::ExtDelegateCall | CallScheme::CallCode)
}

#[derive(Debug, Clone, Copy)]
pub enum DetailedRevertReason {
    CallToNonContract(Address),
    DelegateCallToNonContract(Address),
}

impl fmt::Display for DetailedRevertReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CallToNonContract(addr) => {
                write!(f, "call to non-contract address {addr}")
            }
            Self::DelegateCallToNonContract(addr) => write!(
                f,
                "delegatecall to non-contract address {addr} (usually an unliked library)"
            ),
        }
    }
}

/// An inspector that tracks call context to enhances revert diagnostics.
/// Useful for understanding reverts that are not linked to custom errors or revert strings.
///
/// Supported diagnostics:
///  1. **Non-void call to non-contract address:** the soldity compiler adds some validation to the
///     return data of the call, so despite the call succeeds, as doesn't return data, the
///     validation causes a revert.
///
///     Identified when: a call with non-empty calldata is made to an address without bytecode,
///     followed by an empty revert at the same depth.
///
///  2. **Void call to non-contract address:** in this case the solidity compiler adds some checks
///     before doing the call, so it never takes place.
///
///     Identified when: extcodesize for the target address returns 0 + empty revert at the same
///     depth.
#[derive(Clone, Debug, Default)]
pub struct RevertDiagnostic {
    /// Tracks calls with calldata that target an address without executable code.
    pub non_contract_call: Option<(Address, CallScheme, u64)>,
    /// Tracks EXTCODESIZE checks that target an address without executable code.
    pub non_contract_size_check: Option<(Address, u64)>,
    /// Whether the step opcode is EXTCODESIZE or not.
    pub is_extcodesize_step: bool,
}

impl RevertDiagnostic {
    /// Checks if the `target` address is a precompile for the given `spec_id`.
    fn is_precompile(&self, spec_id: SpecId, target: Address) -> bool {
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id));
        precompiles.contains(&target)
    }

    /// Returns the effective target address whose code would be executed.
    /// For delegate calls, this is the `bytecode_address`. Otherwise, it's the `target_address`.
    fn code_target_address(&self, inputs: &mut CallInputs) -> Address {
        if is_delegatecall(inputs.scheme) {
            inputs.bytecode_address
        } else {
            inputs.target_address
        }
    }

    /// Derives the revert reason based on the cached data. Should only be called after a revert.
    fn reason(&self) -> Option<DetailedRevertReason> {
        if let Some((addr, scheme, _)) = self.non_contract_call {
            let reason = if is_delegatecall(scheme) {
                DetailedRevertReason::DelegateCallToNonContract(addr)
            } else {
                DetailedRevertReason::CallToNonContract(addr)
            };

            return Some(reason);
        }

        if let Some((addr, _)) = self.non_contract_size_check {
            // unknown schema as the call never took place --> output most generic reason
            return Some(DetailedRevertReason::CallToNonContract(addr));
        }

        None
    }

    /// Injects the revert diagnostic into the debug traces. Should only be called after a revert.
    fn handle_revert_diagnostic(&self, interp: &mut Interpreter) {
        if let Some(reason) = self.reason() {
            interp.instruction_result = InstructionResult::Revert;
            interp.next_action = InterpreterAction::Return {
                result: InterpreterResult {
                    output: reason.to_string().abi_encode().into(),
                    gas: interp.gas,
                    result: InstructionResult::Revert,
                },
            };
        }
    }
}

impl<DB: Database + DatabaseExt> Inspector<DB> for RevertDiagnostic {
    /// Tracks the first call with non-zero calldata that targets a non-contract address. Excludes
    /// precompiles and test addresses.
    fn call(&mut self, ctx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let target = self.code_target_address(inputs);

        if IGNORE.contains(&target) || self.is_precompile(ctx.spec_id(), target) {
            return None;
        }

        if let Ok(state) = ctx.code(target) {
            if state.is_empty() && !inputs.input.is_empty() {
                self.non_contract_call = Some((target, inputs.scheme, ctx.journaled_state.depth()));
            }
        }
        None
    }

    /// Handles `REVERT` and `EXTCODESIZE` opcodes for diagnostics.
    ///
    /// When a `REVERT` opcode with zero data size occurs:
    ///  - if `non_contract_call` was set at the current depth, `handle_revert_diagnostic` is
    ///    called. Otherwise, it is cleared.
    ///  - if `non_contract_call` was set at the current depth, `handle_revert_diagnostic` is
    ///    called. Otherwise, it is cleared.
    ///
    /// When an `EXTCODESIZE` opcode occurs:
    ///  - Optimistically caches the target address and current depth in `non_contract_size_check`,
    ///    pending later validation.
    fn step(&mut self, interp: &mut Interpreter, ctx: &mut EvmContext<DB>) {
        // REVERT (offset, size)
        if REVERT == interp.current_opcode() {
            if let Ok(size) = interp.stack().peek(1) {
                if size == U256::ZERO {
                    // Check empty revert with same depth as a non-contract call
                    if let Some((_, _, depth)) = self.non_contract_call {
                        if ctx.journaled_state.depth() == depth {
                            self.handle_revert_diagnostic(interp);
                        } else {
                            self.non_contract_call = None;
                        }
                        return;
                    }

                    // Check empty revert with same depth as a non-contract size check
                    if let Some((_, depth)) = self.non_contract_size_check {
                        if depth == ctx.journaled_state.depth() {
                            self.handle_revert_diagnostic(interp);
                        } else {
                            self.non_contract_size_check = None;
                        }
                    }
                }
            }
        }
        // EXTCODESIZE (address)
        else if EXTCODESIZE == interp.current_opcode() {
            if let Ok(word) = interp.stack().peek(0) {
                let addr = Address::from_word(word.into());
                if IGNORE.contains(&addr) || self.is_precompile(ctx.spec_id(), addr) {
                    return;
                }

                // Optimistically cache --> validated and cleared (if necessary) at `fn step_end()`
                self.non_contract_size_check = Some((addr, ctx.journaled_state.depth()));
                self.is_extcodesize_step = true;
            }
        }
    }

    /// Tracks `EXTCODESIZE` output. If the bytecode size is 0, clears the cache.
    fn step_end(&mut self, interp: &mut Interpreter, _ctx: &mut EvmContext<DB>) {
        if self.is_extcodesize_step {
            if let Ok(size) = interp.stack().peek(0) {
                if size != U256::ZERO {
                    self.non_contract_size_check = None;
                }
            }

            self.is_extcodesize_step = false;
        }
    }
}
