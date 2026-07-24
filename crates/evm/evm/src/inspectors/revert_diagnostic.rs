use alloy_primitives::{Address, U256, map::HashMap};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use foundry_evm_traces::RevertDiagnostic as DetailedRevertReason;
use revm::{
    Inspector,
    bytecode::opcode,
    context::{ContextTr, JournalTr},
    interpreter::{CallInputs, CallOutcome, CallScheme, Interpreter, interpreter_types::Jumps},
};

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// Checks if the call scheme corresponds to any sort of delegate call
pub const fn is_delegatecall(scheme: CallScheme) -> bool {
    matches!(scheme, CallScheme::DelegateCall | CallScheme::CallCode)
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
    non_contract_call: Option<(Address, CallScheme, usize)>,
    /// Tracks EXTCODESIZE checks that target an address without executable code.
    non_contract_size_check: Option<(Address, usize)>,
    /// Whether the step opcode is EXTCODESIZE or not.
    is_extcodesize_step: bool,
    /// Diagnostic detected for the currently executing frame.
    pending: Option<DetailedRevertReason>,
    /// Trace nodes for active frames. `None` means the tracer did not create a node.
    active_trace_nodes: Vec<Option<usize>>,
    /// Presentation-only revert diagnostics keyed by trace node.
    diagnostics: HashMap<usize, DetailedRevertReason>,
}

impl RevertDiagnostic {
    /// Returns the effective target address whose code would be executed.
    /// For delegate calls, this is the `bytecode_address`. Otherwise, it's the `target_address`.
    const fn code_target_address(&self, inputs: &mut CallInputs) -> Address {
        if is_delegatecall(inputs.scheme) { inputs.bytecode_address } else { inputs.target_address }
    }

    /// Derives the revert reason based on the cached data. Should only be called after a revert.
    const fn reason(&self) -> Option<DetailedRevertReason> {
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

    /// Starts tracking a frame before its inspectors can short-circuit execution.
    pub fn frame_start(&mut self) {
        self.active_trace_nodes.push(None);
    }

    /// Associates the active frame with the node created by the tracer.
    pub fn set_trace_node(&mut self, trace_node: usize) {
        let frame = self.active_trace_nodes.last_mut();
        debug_assert!(frame.is_some(), "missing active revert diagnostic frame");
        if let Some(frame) = frame {
            *frame = Some(trace_node);
        }
    }

    /// Finishes tracking a frame and associates any pending diagnostic with its trace node.
    pub fn frame_end(&mut self) {
        let frame = self.active_trace_nodes.pop();
        debug_assert!(frame.is_some(), "revert diagnostic frame stack underflow");
        let diagnostic = self.pending.take();
        if let Some(node_idx) = frame.flatten()
            && let Some(diagnostic) = diagnostic
        {
            self.diagnostics.insert(node_idx, diagnostic);
        }
    }

    /// Consumes the inspector and returns its presentation-only diagnostics.
    pub fn into_diagnostics(self) -> HashMap<usize, DetailedRevertReason> {
        debug_assert!(self.active_trace_nodes.is_empty(), "unclosed revert diagnostic frames");
        self.diagnostics
    }

    /// When a `REVERT` opcode with zero data size occurs, records a diagnostic if a matching
    /// non-contract call or size check was observed at the current depth. Stale observations from
    /// other depths are cleared.
    #[cold]
    fn handle_revert<CTX: ContextTr>(&mut self, interp: &mut Interpreter, ctx: &mut CTX) {
        // REVERT (offset, size)
        if let Ok(size) = interp.stack.peek(1)
            && size.is_zero()
        {
            // Check empty revert with same depth as a non-contract call
            if let Some((_, _, depth)) = self.non_contract_call {
                if ctx.journal_ref().depth() == depth {
                    self.pending = self.reason();
                } else {
                    self.non_contract_call = None;
                }
                return;
            }

            // Check empty revert with same depth as a non-contract size check
            if let Some((_, depth)) = self.non_contract_size_check {
                if depth == ctx.journal_ref().depth() {
                    self.pending = self.reason();
                } else {
                    self.non_contract_size_check = None;
                }
            }
        }
    }

    /// When an `EXTCODESIZE` opcode occurs:
    ///  - Optimistically caches the target address and current depth in `non_contract_size_check`,
    ///    pending later validation.
    #[cold]
    fn handle_extcodesize<CTX: ContextTr>(&mut self, interp: &mut Interpreter, ctx: &mut CTX) {
        // EXTCODESIZE (address)
        if let Ok(word) = interp.stack.peek(0) {
            let addr = Address::from_word(word.into());
            if IGNORE.contains(&addr) || ctx.journal_ref().precompile_addresses().contains(&addr) {
                return;
            }

            // Optimistically cache --> validated and cleared (if necessary) at `fn
            // step_end()`
            self.non_contract_size_check = Some((addr, ctx.journal_ref().depth()));
            self.is_extcodesize_step = true;
        }
    }

    /// Tracks `EXTCODESIZE` output. If the bytecode size is NOT 0, clears the cache.
    #[cold]
    fn handle_extcodesize_output(&mut self, interp: &mut Interpreter) {
        if let Ok(size) = interp.stack.peek(0)
            && size != U256::ZERO
        {
            self.non_contract_size_check = None;
        }

        self.is_extcodesize_step = false;
    }
}

impl<CTX: ContextTr> Inspector<CTX> for RevertDiagnostic {
    /// Tracks the first call with non-zero calldata that targets a non-contract address. Excludes
    /// precompiles and test addresses.
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.input.is_empty() {
            return None;
        }

        let target = self.code_target_address(inputs);

        if IGNORE.contains(&target) || ctx.journal_ref().precompile_addresses().contains(&target) {
            return None;
        }

        if let Ok(state) = ctx.journal_mut().code(target)
            && state.is_empty()
        {
            self.non_contract_call = Some((target, inputs.scheme, ctx.journal_ref().depth()));
        }
        None
    }

    /// Handles `REVERT` and `EXTCODESIZE` opcodes for diagnostics.
    fn step(&mut self, interp: &mut Interpreter, ctx: &mut CTX) {
        match interp.bytecode.opcode() {
            opcode::REVERT => self.handle_revert(interp, ctx),
            opcode::EXTCODESIZE => self.handle_extcodesize(interp, ctx),
            _ => {}
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter, _ctx: &mut CTX) {
        if self.is_extcodesize_step {
            self.handle_extcodesize_output(interp);
        }
    }
}
