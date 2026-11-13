use alloy_primitives::U256;
use evm2::{
    EvmTypesHost, Inspector,
    interpreter::{Host, Interpreter},
};

/// An inspector for Chisel.
#[derive(Clone, Debug, Default)]
pub struct ChiselState {
    /// The PC mapped to the final source statement, if available.
    pub final_pc: Option<usize>,
    /// The state after the final source statement, or at the end of the REPL call if it was not
    /// reached.
    pub state: Option<(Vec<U256>, Vec<u8>)>,
    /// Whether the current instruction matches the final source statement.
    at_final_pc: bool,
}

impl ChiselState {
    /// Create a new Chisel state inspector.
    #[inline]
    pub const fn new(final_pc: Option<usize>) -> Self {
        Self { final_pc, state: None, at_final_pc: false }
    }
}

/// Returns whether `interp` runs the root frame of the transaction.
fn is_root_frame<T: EvmTypesHost>(interp: &mut Interpreter<'_, '_, T>) -> bool {
    interp.message().depth == interp.host().initial_call_depth()
}

impl<T: EvmTypesHost> Inspector<T> for ChiselState {
    fn initialize_interp(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if is_root_frame(interp) {
            self.state = None;
            self.at_final_pc = false;
        }
    }

    #[cold]
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        if is_root_frame(interp) {
            // Read the PC before execution: PUSH and JUMP do not advance it by one byte, and evm2
            // leaves it on an instruction that halts.
            self.at_final_pc = self.final_pc == Some(interp.pc());
        }
    }

    #[cold]
    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        // A source span may be absent or map to an instruction that never executes. In that case,
        // capture the completed root frame, not PC zero or a nested call's stack and memory.
        // The result is only an error after the instruction that halts the frame.
        if is_root_frame(interp)
            && (self.at_final_pc || (self.state.is_none() && interp.result().is_err()))
        {
            self.state =
                Some((interp.stack().as_slice().to_vec(), interp.memory().as_slice().to_vec()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{Address, hex};
    use evm2::{
        bytecode::Bytecode,
        evm::{AccountInfo, InMemoryDB},
    };
    use foundry_evm_core::evm::{
        EthEvmNetwork, EvmEnv, EvmFactoryFor, FoundryEvmFactory, FoundryTxConversion, SpecFor,
        TransactionRequestFor, TypesFor,
    };

    fn execute(inspector: ChiselState, code: &str) -> ChiselState {
        let target = Address::with_last_byte(0x40);
        let child = Address::with_last_byte(0x20);
        let mut db = InMemoryDB::default();
        for (address, code) in [(target, code), (child, "61cafe5f52600700")] {
            db.insert_account_info(
                &address,
                AccountInfo::default()
                    .with_code(Bytecode::new_legacy(hex::decode(code).unwrap().into())),
            );
        }
        let factory = EvmFactoryFor::<EthEvmNetwork>::default();
        let spec = SpecFor::<EthEvmNetwork>::default();
        let mut evm =
            factory.create_evm(db, EvmEnv::new(spec, factory.version(spec, 1), Default::default()));
        evm.set_inspector(inspector);
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
        assert!(result.status);
        *evm.clear_inspector_as::<ChiselState>().unwrap()
    }

    #[test]
    fn captures_final_state_without_an_executed_source_match() {
        for final_pc in [None, Some(100)] {
            // MSTORE(0, 0x1337), PUSH1 42, STOP.
            let inspector = execute(ChiselState::new(final_pc), "6113375f52602a00");
            assert_eq!(
                inspector.state,
                Some((vec![U256::from(42)], U256::from(0x1337).to_be_bytes::<32>().to_vec()))
            );
        }
    }

    #[test]
    fn preserves_source_snapshot_after_push_and_jump() {
        for (code, final_pc, expected_stack) in [
            // A valid source match at PC zero must still work, including PUSH immediates.
            ("6113375f52602a00", 0, vec![U256::from(0x1337)]),
            // PUSH1 42, PUSH1 6, JUMP, INVALID, JUMPDEST, POP, STOP.
            ("602a600656fe5b5000", 4, vec![U256::from(42)]),
        ] {
            let inspector = execute(ChiselState::new(Some(final_pc)), code);
            assert_eq!(inspector.state, Some((expected_stack, vec![])));
        }
    }

    #[test]
    fn nested_calls_do_not_replace_root_state() {
        for (final_pc, expected_stack) in [(None, vec![U256::from(42)]), (Some(4), vec![])] {
            // MSTORE(0, 0x1337), CALL child, POP success, PUSH1 42, STOP. The child also executes
            // MSTORE at PC 4, but writes 0xcafe and leaves 7 on its stack.
            let inspector =
                execute(ChiselState::new(final_pc), "6113375f525f5f5f5f5f60205af150602a00");
            assert_eq!(
                inspector.state,
                Some((expected_stack, U256::from(0x1337).to_be_bytes::<32>().to_vec()))
            );
        }
    }

    #[test]
    fn resets_snapshot_between_executions() {
        let inspector = execute(ChiselState::new(Some(4)), "6113375f52602a00");
        // The second execution never reaches PC 4 and must use its own terminal snapshot.
        let inspector = execute(inspector, "602b00");
        assert_eq!(inspector.state, Some((vec![U256::from(43)], vec![])));
    }
}
