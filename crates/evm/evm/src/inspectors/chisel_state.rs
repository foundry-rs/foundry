use alloy_primitives::U256;
use revm::{
    Inspector,
    context::{ContextTr, JournalTr},
    interpreter::{
        Interpreter,
        interpreter_types::{Jumps, LoopControl},
    },
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

impl<CTX: ContextTr> Inspector<CTX> for ChiselState {
    fn initialize_interp(&mut self, _interpreter: &mut Interpreter, context: &mut CTX) {
        if context.journal().depth() == 1 {
            self.state = None;
            self.at_final_pc = false;
        }
    }

    #[cold]
    fn step(&mut self, interpreter: &mut Interpreter, context: &mut CTX) {
        if context.journal().depth() == 1 {
            // Read the PC before execution: PUSH and JUMP do not advance it by one byte.
            self.at_final_pc = self.final_pc == Some(interpreter.bytecode.pc());
        }
    }

    #[cold]
    fn step_end(&mut self, interpreter: &mut Interpreter, context: &mut CTX) {
        // A source span may be absent or map to an instruction that never executes. In that case,
        // capture the completed root frame, not PC zero or a nested call's stack and memory.
        // `instruction_result` excludes CALL/CREATE yields, which do not end the REPL call.
        if context.journal().depth() == 1
            && (self.at_final_pc
                || (self.state.is_none() && interpreter.bytecode.instruction_result().is_some()))
        {
            self.state = Some((
                interpreter.stack.data().clone(),
                interpreter.memory.context_memory().to_vec(),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, TxKind, hex};
    use revm::{
        Context, InspectEvm, MainBuilder, MainContext,
        context::TxEnv,
        database::InMemoryDB,
        state::{AccountInfo, Bytecode},
    };

    fn execute(inspector: &mut ChiselState, code: &str) {
        let target = Address::with_last_byte(0x40);
        let child = Address::with_last_byte(0x20);
        let mut db = InMemoryDB::default();
        for (address, code) in [(target, code), (child, "61cafe5f52600700")] {
            db.insert_account_info(
                address,
                AccountInfo::from_bytecode(Bytecode::new_raw(hex::decode(code).unwrap().into())),
            );
        }
        let result = Context::mainnet()
            .with_db(db)
            .build_mainnet_with_inspector(inspector)
            .inspect_one_tx(
                TxEnv::builder()
                    .caller(Address::with_last_byte(0x30))
                    .kind(TxKind::Call(target))
                    .gas_limit(1_000_000)
                    .build()
                    .unwrap(),
            )
            .unwrap();
        assert!(result.is_success());
    }

    #[test]
    fn captures_final_state_without_an_executed_source_match() {
        for final_pc in [None, Some(100)] {
            let mut inspector = ChiselState::new(final_pc);
            // MSTORE(0, 0x1337), PUSH1 42, STOP.
            execute(&mut inspector, "6113375f52602a00");
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
            let mut inspector = ChiselState::new(Some(final_pc));
            execute(&mut inspector, code);
            assert_eq!(inspector.state, Some((expected_stack, vec![])));
        }
    }

    #[test]
    fn nested_calls_do_not_replace_root_state() {
        for (final_pc, expected_stack) in [(None, vec![U256::from(42)]), (Some(4), vec![])] {
            let mut inspector = ChiselState::new(final_pc);
            // MSTORE(0, 0x1337), CALL child, POP success, PUSH1 42, STOP. The child also executes
            // MSTORE at PC 4, but writes 0xcafe and leaves 7 on its stack.
            execute(&mut inspector, "6113375f525f5f5f5f5f60205af150602a00");
            assert_eq!(
                inspector.state,
                Some((expected_stack, U256::from(0x1337).to_be_bytes::<32>().to_vec()))
            );
        }
    }

    #[test]
    fn resets_snapshot_between_executions() {
        let mut inspector = ChiselState::new(Some(4));
        execute(&mut inspector, "6113375f52602a00");
        // The second execution never reaches PC 4 and must use its own terminal snapshot.
        execute(&mut inspector, "602b00");
        assert_eq!(inspector.state, Some((vec![U256::from(43)], vec![])));
    }
}
