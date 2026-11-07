use alloy_primitives::{Address, map::HashMap};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use foundry_evm_traces::RevertDiagnostic as DetailedRevertReason;
use revm::{
    Inspector,
    bytecode::opcode,
    context::{ContextTr, JournalTr},
    interpreter::{CallInputs, CallOutcome, CallScheme, Interpreter, interpreter_types::Jumps},
};

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// An `EXTCODESIZE` probe of an address without executable code.
///
/// The probe is cached optimistically when the opcode starts, and validated against the opcode's
/// output when it ends.
#[derive(Clone, Copy, Debug)]
enum SizeCheck {
    /// `EXTCODESIZE` is executing; its output is validated in `step_end`.
    Pending { addr: Address, depth: usize },
    /// `EXTCODESIZE` did not report any code for the address.
    NonContract { addr: Address, depth: usize },
}

impl SizeCheck {
    const fn target(self) -> (Address, usize) {
        match self {
            Self::Pending { addr, depth } | Self::NonContract { addr, depth } => (addr, depth),
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
    non_contract_call: Option<(Address, CallScheme, usize)>,
    /// Tracks EXTCODESIZE checks that target an address without executable code.
    non_contract_size_check: Option<SizeCheck>,
    /// Diagnostic detected for the currently executing frame.
    pending: Option<DetailedRevertReason>,
    /// Trace nodes for active frames. `None` means the tracer did not create a node.
    active_trace_nodes: Vec<Option<usize>>,
    /// Presentation-only revert diagnostics keyed by trace node.
    diagnostics: HashMap<usize, DetailedRevertReason>,
}

impl RevertDiagnostic {
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

        if let Some(check) = self.non_contract_size_check {
            let (addr, _) = check.target();
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
            if let Some(check) = self.non_contract_size_check {
                let (_, depth) = check.target();
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
            self.non_contract_size_check =
                Some(SizeCheck::Pending { addr, depth: ctx.journal_ref().depth() });
        }
    }

    /// Validates a pending `EXTCODESIZE` probe against its output: a non-zero size clears it,
    /// otherwise the address is confirmed to have no code and supersedes any earlier non-contract
    /// call.
    #[cold]
    fn handle_extcodesize_output(&mut self, interp: &mut Interpreter, addr: Address, depth: usize) {
        self.non_contract_size_check = match interp.stack.peek(0) {
            Ok(size) if !size.is_zero() => None,
            _ => {
                self.non_contract_call = None;
                Some(SizeCheck::NonContract { addr, depth })
            }
        };
    }
}

impl<CTX: ContextTr> Inspector<CTX> for RevertDiagnostic {
    /// Tracks the latest call with non-zero calldata that targets a non-contract address. Excludes
    /// precompiles and test addresses.
    fn call(&mut self, ctx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.input.is_empty() {
            return None;
        }

        // Delegate calls execute the callee's code in the caller's storage context.
        let target = if is_delegatecall(inputs.scheme) {
            inputs.bytecode_address
        } else {
            inputs.transfer_to()
        };

        if IGNORE.contains(&target) || ctx.journal_ref().precompile_addresses().contains(&target) {
            return None;
        }

        if let Ok(state) = ctx.journal_mut().code(target)
            && state.is_empty()
        {
            self.non_contract_call = Some((target, inputs.scheme, ctx.journal_ref().depth()));
            // The latest observation explains a following empty revert.
            self.non_contract_size_check = None;
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
        if let Some(SizeCheck::Pending { addr, depth }) = self.non_contract_size_check {
            self.handle_extcodesize_output(interp, addr, depth);
        }
    }
}

/// Checks if the call scheme corresponds to any sort of delegate call
pub const fn is_delegatecall(scheme: CallScheme) -> bool {
    matches!(scheme, CallScheme::DelegateCall | CallScheme::CallCode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{TxKind, hex};
    use revm::{
        Context, InspectEvm, MainBuilder, MainContext,
        context::TxEnv,
        database::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    #[test]
    fn latest_non_contract_observation_explains_empty_revert() {
        let target = Address::with_last_byte(0x40);
        let probed = Address::with_last_byte(0xe2);
        let mut db = InMemoryDB::default();
        let code = hex::decode(concat!(
            "5f5f60015f5f60e15af150", // CALL(gas, 0xe1, 0, 0, 1, 0, 0) to an address without code.
            "60e23b50",               // EXTCODESIZE(0xe2), which has no code either.
            "5f5ffd"                  // REVERT(0, 0).
        ))
        .unwrap();
        db.insert_account_info(target, AccountInfo::from_bytecode(Bytecode::new_raw(code.into())));
        let mut diagnostic = RevertDiagnostic::default();
        let result = Context::mainnet()
            .with_db(db)
            .build_mainnet_with_inspector(&mut diagnostic)
            .inspect_one_tx(
                TxEnv::builder()
                    .caller(Address::with_last_byte(0x30))
                    .kind(TxKind::Call(target))
                    .gas_limit(1_000_000)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        assert!(!result.is_success());
        assert!(
            matches!(
                diagnostic.pending,
                Some(DetailedRevertReason::CallToNonContract(addr)) if addr == probed
            ),
            "{:?}",
            diagnostic.pending
        );
    }
}
