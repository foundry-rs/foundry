use alloy_primitives::{Address, map::HashMap};
use evm2::{
    EvmTypes, Inspector, InspectorConfig,
    interpreter::{Interpreter, Message, MessageKind, MessageResult, op},
};
use foundry_evm_core::constants::{CHEATCODE_ADDRESS, HARDHAT_CONSOLE_ADDRESS};
use foundry_evm_traces::RevertDiagnostic as DetailedRevertReason;

const IGNORE: [Address; 2] = [HARDHAT_CONSOLE_ADDRESS, CHEATCODE_ADDRESS];

/// An `EXTCODESIZE` probe of an address without executable code.
///
/// The probe is cached optimistically when the opcode starts, and validated against the opcode's
/// output when it ends.
#[derive(Clone, Copy, Debug)]
enum SizeCheck {
    /// `EXTCODESIZE` is executing; its output is validated in `step_end`.
    Pending { addr: Address, depth: u16 },
    /// `EXTCODESIZE` did not report any code for the address.
    NonContract { addr: Address, depth: u16 },
}

impl SizeCheck {
    const fn target(self) -> (Address, u16) {
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
    non_contract_call: Option<(Address, MessageKind, u16)>,
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
    fn handle_revert<T: EvmTypes>(&mut self, interp: &Interpreter<'_, '_, T>) {
        let depth = interp.message().depth;
        // REVERT (offset, size)
        if let Some(size) = interp.stack().peek(1)
            && size.is_zero()
        {
            // Check empty revert with same depth as a non-contract call
            if let Some((_, _, call_depth)) = self.non_contract_call {
                if depth == call_depth {
                    self.pending = self.reason();
                } else {
                    self.non_contract_call = None;
                }
                return;
            }

            // Check empty revert with same depth as a non-contract size check
            if let Some(check) = self.non_contract_size_check {
                let (_, check_depth) = check.target();
                if check_depth == depth {
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
    fn handle_extcodesize<T: EvmTypes>(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        // EXTCODESIZE (address)
        if let Some(word) = interp.stack().peek(0) {
            let addr = Address::from_word(word.into());
            let depth = interp.message().depth;
            if IGNORE.contains(&addr) || interp.host().precompiles().contains(&addr) {
                return;
            }

            // Optimistically cache --> validated and cleared (if necessary) at `fn
            // step_end()`
            self.non_contract_size_check = Some(SizeCheck::Pending { addr, depth });
        }
    }

    /// Validates a pending `EXTCODESIZE` probe against its output: a non-zero size clears it,
    /// otherwise the address is confirmed to have no code and supersedes any earlier non-contract
    /// call.
    #[cold]
    fn handle_extcodesize_output<T: EvmTypes>(
        &mut self,
        interp: &Interpreter<'_, '_, T>,
        addr: Address,
        depth: u16,
    ) {
        self.non_contract_size_check = match interp.stack().peek(0) {
            Some(size) if !size.is_zero() => None,
            _ => {
                self.non_contract_call = None;
                Some(SizeCheck::NonContract { addr, depth })
            }
        };
    }
}

impl<T: EvmTypes> Inspector<T> for RevertDiagnostic {
    fn inspector_config(&self) -> InspectorConfig {
        InspectorConfig::new().with_steps([op::REVERT, op::EXTCODESIZE].into_iter().collect())
    }

    /// Tracks the latest call with non-zero calldata that targets a non-contract address. Excludes
    /// precompiles and test addresses.
    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        if message.input.is_empty() {
            return None;
        }

        // Delegate calls execute the callee's code in the caller's storage context.
        let target =
            if is_delegatecall(message.kind) { message.code_address } else { message.destination };

        let host = interp.host();
        if IGNORE.contains(&target) || host.precompiles().contains(&target) {
            return None;
        }

        if let Ok(info) = host.state_mut().account_info_untracked(&target)
            && info.is_none_or(|info| info.is_empty_code_hash())
        {
            // The revert happens in the calling frame. The hook can run on a frame EVM whose
            // interpreter is not the caller's, so take the depth from the message.
            self.non_contract_call = Some((target, message.kind, message.depth.saturating_sub(1)));
            // The latest observation explains a following empty revert.
            self.non_contract_size_check = None;
        }
        None
    }

    /// Handles `REVERT` and `EXTCODESIZE` opcodes for diagnostics.
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        match interp.opcode() {
            op::REVERT => self.handle_revert(interp),
            op::EXTCODESIZE => self.handle_extcodesize(interp),
            _ => {}
        }
    }

    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if let Some(SizeCheck::Pending { addr, depth }) = self.non_contract_size_check {
            self.handle_extcodesize_output(interp, addr, depth);
        }
    }
}

/// Checks if the call scheme corresponds to any sort of delegate call
pub const fn is_delegatecall(kind: MessageKind) -> bool {
    matches!(kind, MessageKind::DelegateCall | MessageKind::CallCode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::TransactionBuilder;
    use alloy_primitives::hex;
    use evm2::{
        bytecode::Bytecode,
        evm::{AccountInfo, InMemoryDB},
    };
    use foundry_evm_core::evm::{
        EthEvmNetwork, EvmEnv, EvmFactoryFor, FoundryEvmFactory, FoundryTxConversion, SpecFor,
        TransactionRequestFor, TypesFor,
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
        db.insert_account_info(
            &target,
            AccountInfo::default().with_code(Bytecode::new_legacy(code.into())),
        );
        let factory = EvmFactoryFor::<EthEvmNetwork>::default();
        let spec = SpecFor::<EthEvmNetwork>::default();
        let mut evm =
            factory.create_evm(db, EvmEnv::new(spec, factory.version(spec, 1), Default::default()));
        evm.set_inspector(RevertDiagnostic::default());
        let request = TransactionRequestFor::<EthEvmNetwork>::default()
            .with_from(Address::with_last_byte(0x30))
            .with_to(target)
            .with_nonce(0)
            .with_chain_id(1)
            .with_gas_limit(1_000_000)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0);
        let tx = TypesFor::<EthEvmNetwork>::tx_from_request(request).unwrap();
        let result = evm.transact(&tx).unwrap().commit();
        let diagnostic = evm.clear_inspector_as::<RevertDiagnostic>().unwrap();
        assert!(!result.status);
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
