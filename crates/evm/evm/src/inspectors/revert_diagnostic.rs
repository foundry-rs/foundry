use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_core::{
    backend::DatabaseExt,
    constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS},
    decode::DetailedRevertReason,
};
use revm::{
    interpreter::{
        opcode::{EXTCODESIZE, REVERT},
        CallInputs, CallOutcome, CallScheme, InstructionResult, Interpreter,
    },
    precompile::{PrecompileSpecId, Precompiles},
    primitives::SpecId,
    Database, EvmContext, Inspector,
};

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// Checks if the call scheme corresponds to any sort of delegate call
pub fn is_delegatecall(scheme: CallScheme) -> bool {
    matches!(scheme, CallScheme::DelegateCall | CallScheme::ExtDelegateCall | CallScheme::CallCode)
}

/// An inspector that tracks call context to enhances revert diagnostics.
/// Useful for understanding reverts that are not linked to custom errors or revert strings.
///
/// Supported diagnostics:
///  1. **Non-void call to non-contract address:** the soldity compiler adds some validation to the
///     return data of the call, so despite the call succeeds, as doesn't return data, the
///     validation causes a revert.
///
///     Identified when: a call to an address with no code and non-empty calldata is made, followed
///     by an empty revert at the same depth
///
///  2. **Void call to non-contract address:** in this case the solidity compiler adds some checks
///     before doing the call, so it never takes place.
///
///     Identified when: extcodesize for the target address returns 0 + empty revert at the same
///     depth
#[derive(Clone, Debug, Default)]
pub struct RevertDiagnostic {
    /// Tracks calls with calldata that target an address without executable code.
    pub non_contract_call: Option<(Address, CallScheme, u64)>,
    /// Tracks EXTCODESIZE checks that target an address without executable code.
    pub non_contract_size_check: Option<(Address, u64)>,
    /// Whether the step opcode is EXTCODESIZE or not.
    pub is_extcodesize_step: bool,
    /// Tracks whether a failed call has been spotted or not.
    pub reverted: bool,
}

impl RevertDiagnostic {
    /// Checks if the `target` address is a precompile for the given `spec_id`.
    fn is_precompile(&self, spec_id: SpecId, target: Address) -> bool {
        let precompiles = Precompiles::new(PrecompileSpecId::from_spec_id(spec_id));
        precompiles.contains(&target)
    }

    /// Returns the effective target address whose code would be executed.
    /// For delegate calls, this is the `bytecode_address`. Otherwise, it's the `target_address`.
    pub fn no_code_target_address(&self, inputs: &mut CallInputs) -> Address {
        if is_delegatecall(inputs.scheme) {
            inputs.bytecode_address
        } else {
            inputs.target_address
        }
    }

    /// Derives the revert reason based on the cached information.
    pub fn reason(&self) -> Option<DetailedRevertReason> {
        if self.reverted {
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
        }

        None
    }
}

impl<DB: Database + DatabaseExt> Inspector<DB> for RevertDiagnostic {
    /// Tracks the first call with non-zero calldata that targets a non-contract address. Excludes
    /// precompiles and test addresses.
    fn call(&mut self, ctx: &mut EvmContext<DB>, inputs: &mut CallInputs) -> Option<CallOutcome> {
        let target = self.no_code_target_address(inputs);

        if IGNORE.contains(&target) || self.is_precompile(ctx.spec_id(), target) {
            return None;
        }

        if let Ok(state) = ctx.code(target) {
            if state.is_empty() && !inputs.input.is_empty() && !self.reverted {
                self.non_contract_call = Some((target, inputs.scheme, ctx.journaled_state.depth()));
            }
        }
        None
    }

    /// If a `non_contract_call` was previously recorded, will check if the call reverted without
    /// data at the same depth. If so, flags `reverted` as `true`.
    fn call_end(
        &mut self,
        ctx: &mut EvmContext<DB>,
        _inputs: &CallInputs,
        outcome: CallOutcome,
    ) -> CallOutcome {
        if let Some((_, _, depth)) = self.non_contract_call {
            if outcome.result.result == InstructionResult::Revert &&
                outcome.result.output == Bytes::new() &&
                ctx.journaled_state.depth() == depth - 1
            {
                self.reverted = true
            };
        }

        outcome
    }

    /// When the current opcode is `EXTCODESIZE`:
    ///    - Tracks addresses being checked and the current depth (if not ignored or a precompile)
    ///      on `non_contract_size_check`.
    ///
    /// When `non_contract_size_check` is `Some`:
    ///    - If the call stack depth changes clears the cached data.
    ///    - If the current opcode is `REVERT` and its size is zero, sets `reverted` to `true`.
    fn step(&mut self, interp: &mut Interpreter, ctx: &mut EvmContext<DB>) {
        if let Some((_, depth)) = self.non_contract_size_check {
            if depth != ctx.journaled_state.depth() {
                self.non_contract_size_check = None;
            }

            if REVERT == interp.current_opcode() {
                if let Ok(size) = interp.stack().peek(1) {
                    if size == U256::ZERO {
                        self.reverted = true;
                    }
                }
            }

            return;
        }

        if EXTCODESIZE == interp.current_opcode() {
            if let Ok(word) = interp.stack().peek(0) {
                let addr = Address::from_word(word.into());
                if IGNORE.contains(&addr) || self.is_precompile(ctx.spec_id(), addr) {
                    return;
                }

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
