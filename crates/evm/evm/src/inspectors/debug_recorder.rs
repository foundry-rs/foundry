//! Records the debugger's opcode steps straight into a SolDB step table.

use alloy_primitives::U256;
use evm2::{
    EvmTypes, Inspector,
    evm::JournalEntry,
    interpreter::{InstrStop, Interpreter, Message, MessageResult, opcode::OpCode},
};
use foundry_evm_traces::debug::{DebugRecording, DebugStepSink, RecordedFrame, StepConverter};
use soldb_core::{EvmWord, StepRecord, StepTable};
use std::{fmt, sync::Arc};

/// The most stack words one instruction reads: `SWAP16` reads 17.
const MAX_INPUTS: usize = 17;

/// Records every opcode step of the debugged calls into a [`StepTable`], with the steps that
/// each call and creation spans.
///
/// The steps are the ones `TracingInspector` records in debug mode, converted as the SolDB
/// frontend converts them, but without the per-step `CallTraceStep` and its full stack copy. The
/// inspector stack calls this recorder next to the tracer, so its frames are the tracer's nodes
/// in the same order.
pub struct DebugRecorder {
    sink: DebugStepSink,
    /// The sink's table while this recorder appends to it.
    table: Option<StepTable>,
    /// The first step this recorder appended.
    first_step: usize,
    /// The steps of each call trace node, by node index.
    frames: Vec<RecordedFrame>,
    /// The frames that run now, innermost last.
    open: Vec<OpenFrame>,
    /// The step that started and did not end yet.
    pending: Option<PendingStep>,
    /// Converts the return data and storage of the steps.
    converter: StepConverter,
}

/// A frame that runs now.
struct OpenFrame {
    /// The index of the frame in `frames`.
    frame: usize,
    /// The range the previous opcode may have written, applied when this frame resumes.
    write: Option<std::ops::Range<usize>>,
    /// The memory size already recorded for this frame.
    memory_size: usize,
}

/// What a step records when it starts.
struct PendingStep {
    pc: u64,
    op: OpCode,
    gas: u64,
    depth: u64,
    #[cfg(test)]
    memory: Vec<u8>,
    return_data: Arc<str>,
    /// The stack height before the step.
    height: usize,
    /// The top words the step can pop or change, bottom first.
    inputs: [U256; MAX_INPUTS],
    inputs_len: usize,
    /// The journal length before the step.
    journal_len: usize,
}

impl DebugRecorder {
    /// A recorder that appends to `sink`.
    pub fn new(sink: DebugStepSink) -> Self {
        Self {
            sink,
            table: None,
            first_step: 0,
            // The arena of a new tracer starts with an empty root node.
            frames: vec![RecordedFrame::default()],
            open: Vec::new(),
            pending: None,
            converter: StepConverter::default(),
        }
    }

    /// The steps this recorder appended, by call trace node.
    pub fn finish(&mut self) -> DebugRecording {
        let end = self.table.as_ref().map_or(self.first_step, StepTable::len);
        DebugRecording {
            sink: self.sink.clone(),
            steps: self.first_step..end,
            frames: std::mem::take(&mut self.frames),
        }
    }

    /// The table to append to, taken from the sink on first use.
    fn table(&mut self) -> &mut StepTable {
        if self.table.is_none() {
            // A recorder of another stack that has not finished holds the shared table, so this
            // one records into a table of its own.
            let table = self.sink.take().unwrap_or_else(|| {
                self.sink = DebugStepSink::new();
                self.sink.take().unwrap_or_default()
            });
            self.first_step = table.len();
            self.table = Some(table);
        }
        self.table.as_mut().unwrap()
    }

    /// Opens a frame at journal depth `depth`, as the tracer opens a node.
    fn enter(&mut self, depth: usize) {
        let entry_step = self.table().len() - self.first_step;
        let frame = RecordedFrame { depth, entry_step, exit_step: entry_step };
        // The tracer replaces the root node with the call at depth zero.
        let index = if depth == 0 {
            self.frames[0] = frame;
            0
        } else {
            self.frames.push(frame);
            self.frames.len() - 1
        };
        self.table().start_memory_frame();
        self.open.push(OpenFrame { frame: index, write: None, memory_size: 0 });
    }

    /// Closes the innermost frame.
    fn exit(&mut self) {
        debug_assert!(self.pending.is_none(), "a step did not end before its frame");
        self.pending = None;
        let exit_step = self.table().len() - self.first_step;
        if let Some(open) = self.open.pop() {
            self.frames[open.frame].exit_step = exit_step;
            self.table().end_memory_frame();
        }
    }

    /// Starts a step.
    fn start_step<T: EvmTypes>(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let Some(open) = self.open.last_mut() else { return };
        let op = OpCode::new_or_unknown(interp.opcode());

        // CALL output is copied after the child's call_end hook. Record it when the caller
        // resumes, along with ordinary writes, before taking this step's memory reference.
        let current = interp.memory().as_slice();
        let table = self.table.as_mut().expect("a frame is open");
        if let Some(range) = open.write.take() {
            let start = range.start.min(current.len());
            let end = range.end.min(current.len());
            table
                .write_memory(start as u64, &current[start..end], current.len() as u64)
                .expect("the write is bounded by the interpreter's memory");
        } else if open.memory_size != current.len() {
            table
                .write_memory(0, &[], current.len() as u64)
                .expect("EVM memory only grows within a frame");
        }
        open.memory_size = current.len();
        open.write = memory_write(op, interp.stack().as_slice());
        #[cfg(test)]
        let memory = current.to_vec();

        let return_data = self.converter.return_data(interp.return_data());

        // An instruction changes only the words it reads, so the stack before it is what stays
        // below them after it, and these.
        let stack = interp.stack();
        let stack = stack.as_slice();
        let read = if op.is_valid() { op.inputs() as usize } else { 0 };
        let inputs_len = read.min(stack.len()).min(MAX_INPUTS);
        let mut inputs = [U256::ZERO; MAX_INPUTS];
        inputs[..inputs_len].copy_from_slice(&stack[stack.len() - inputs_len..]);

        let depth = self.frames[open.frame].depth as u64 + 1;
        let height = stack.len();
        let pc = interp.pc() as u64;
        let gas = interp.gas().remaining();
        let journal_len = interp.host().state().journal().len();
        self.pending = Some(PendingStep {
            pc,
            op,
            gas,
            depth,
            #[cfg(test)]
            memory,
            return_data,
            height,
            inputs,
            inputs_len,
            journal_len,
        });
    }

    /// Ends the pending step after its instruction ran.
    fn end_step<T: EvmTypes>(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        let Some(step) = self.pending.take() else { return };
        let status = interp.result().err();
        let had_value = if step.op == OpCode::SSTORE {
            let journal = interp.host().state().journal();
            (journal.len() != step.journal_len)
                .then(|| match journal.last() {
                    Some(JournalEntry::StorageChange { previous, .. }) => Some(*previous),
                    _ => None,
                })
                .flatten()
        } else {
            None
        };
        // As the tracer: the reentrancy sentry halts before any charge.
        let gas_cost = if interp.result() == Err(InstrStop::ReentrancySentryOOG) {
            0
        } else {
            interp.step_cost()
        };
        let after = interp.stack();
        let after = after.as_slice();
        // The frame runs another step unless this one ended it; that step starts with the value
        // a load pushed on top.
        let loaded = status.is_none().then(|| after.last().copied()).flatten();
        let kept = &after[..(step.height - step.inputs_len).min(after.len())];
        self.append(step, kept, status, gas_cost, loaded, had_value);
    }

    /// Ends the pending step of the instruction that starts a call or creation.
    ///
    /// evm2 ends such a step after its frame returns, but the step comes before the frame's
    /// steps, so it ends when the frame starts, with the gas the caller charged for it.
    fn end_step_before_frame<T: EvmTypes>(&mut self, interp: &Interpreter<'_, '_, T>) {
        let Some(step) = self.pending.take() else { return };
        let gas_cost = step.gas.saturating_sub(interp.gas().remaining());
        let stack = interp.stack();
        let stack = stack.as_slice();
        let kept = &stack[..(step.height - step.inputs_len).min(stack.len())];
        self.append(step, kept, None, gas_cost, None, None);
    }

    /// Ends the pending step if an inspector halted the frame before its instruction ran.
    pub fn end_step_not_run<T: EvmTypes>(&mut self, interp: &Interpreter<'_, '_, T>) {
        let Some(step) = self.pending.take() else { return };
        let stack = interp.stack();
        let stack = stack.as_slice();
        let kept = &stack[..(step.height - step.inputs_len).min(stack.len())];
        self.append(step, kept, None, 0, None, None);
    }

    /// Appends `step`, whose stack is `kept` and its inputs.
    ///
    /// `loaded` is the value an `SLOAD` or `TLOAD` pushed, if the frame went on, and `had_value`
    /// the value an `SSTORE` overwrote, if the journal recorded it.
    fn append(
        &mut self,
        step: PendingStep,
        kept: &[U256],
        status: Option<InstrStop>,
        gas_cost: u64,
        loaded: Option<U256>,
        had_value: Option<U256>,
    ) {
        #[cfg(test)]
        let expected_memory = step.memory;
        let PendingStep { pc, op, gas, depth, return_data, inputs, inputs_len, .. } = step;
        let inputs = &inputs[..inputs_len];
        let storage = self.converter.storage(op.get(), inputs, loaded, had_value);
        let error =
            status.filter(|status| !status.is_success()).map(|status| format!("{status:?}"));
        let table = self.table.as_mut().expect("a frame is open");
        table.append(
            StepRecord {
                pc,
                op: op.get(),
                gas,
                gas_cost,
                depth,
                error,
                memory: None,
                storage: Some(storage.storage),
                storage_diff: storage.storage_diff,
                transient_storage: storage.transient_storage,
                return_data: Some(return_data),
            },
            kept.iter().chain(inputs).map(|word| EvmWord(word.to_be_bytes())),
        );
        #[cfg(test)]
        assert_eq!(
            table.step(table.len() - 1).memory().as_deref(),
            Some(expected_memory.as_slice()),
            "recorded memory differs at pc {pc}, opcode {op}"
        );
    }
}

impl Default for DebugRecorder {
    fn default() -> Self {
        Self::new(DebugStepSink::new())
    }
}

/// A clone records the next call into the same sink, from an empty state.
impl Clone for DebugRecorder {
    fn clone(&self) -> Self {
        Self::new(self.sink.clone())
    }
}

/// Gives the table back to the sink.
impl Drop for DebugRecorder {
    fn drop(&mut self) {
        if let Some(table) = self.table.take() {
            self.sink.put(table);
        }
    }
}

impl fmt::Debug for DebugRecorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugRecorder")
            .field("frames", &self.frames.len())
            .field("steps", &self.table.as_ref().map(StepTable::len))
            .finish_non_exhaustive()
    }
}

impl<T: EvmTypes> Inspector<T> for DebugRecorder {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.start_step(interp);
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
        self.end_step(interp);
    }

    fn call(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.end_step_before_frame(interp);
        self.enter(usize::from(message.depth));
        None
    }

    fn call_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        _message: &Message<T>,
        _result: &mut MessageResult<T>,
    ) {
        self.exit();
    }

    fn create(
        &mut self,
        interp: &mut Interpreter<'_, '_, T>,
        message: &mut Message<T>,
    ) -> Option<MessageResult<T>> {
        self.end_step_before_frame(interp);
        self.enter(usize::from(message.depth));
        None
    }

    fn create_end(
        &mut self,
        _interp: &mut Interpreter<'_, '_, T>,
        _message: &Message<T>,
        _result: &mut MessageResult<T>,
    ) {
        self.exit();
    }
}

/// The destination a memory-writing opcode takes from its pre-execution stack.
/// Memory-reading opcodes can expand memory but leave all new bytes zero.
fn memory_write(op: OpCode, stack: &[U256]) -> Option<std::ops::Range<usize>> {
    let word = |depth: usize| -> Option<usize> {
        stack.get(stack.len().checked_sub(depth + 1)?)?.try_into().ok()
    };
    let (offset, length) = match op {
        OpCode::MSTORE => (word(0)?, 32),
        OpCode::MSTORE8 => (word(0)?, 1),
        OpCode::CALLDATACOPY | OpCode::CODECOPY | OpCode::RETURNDATACOPY | OpCode::MCOPY => {
            (word(0)?, word(2)?)
        }
        OpCode::EXTCODECOPY => (word(1)?, word(3)?),
        OpCode::CALL | OpCode::CALLCODE => (word(5)?, word(6)?),
        OpCode::DELEGATECALL | OpCode::STATICCALL => (word(4)?, word(5)?),
        _ => return None,
    };
    Some(offset..offset.saturating_add(length))
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
        EthEvmNetwork, EvmEnv, EvmFactoryFor, FoundryEvmFactory, FoundryEvmNetwork, SpecFor,
        TransactionRequestFor,
    };

    /// Runs the recorder and records the interpreter's memory and stack before each step.
    #[derive(Default)]
    struct Snapshots {
        recorder: DebugRecorder,
        steps: Vec<(Vec<u8>, Vec<EvmWord>)>,
    }

    impl<T: EvmTypes> Inspector<T> for Snapshots {
        fn step(&mut self, interp: &mut Interpreter<'_, '_, T>) {
            self.steps.push((
                interp.memory().as_slice().to_vec(),
                interp.stack().as_slice().iter().map(|word| EvmWord(word.to_be_bytes())).collect(),
            ));
            Inspector::<T>::step(&mut self.recorder, interp);
        }

        fn step_end(&mut self, interp: &mut Interpreter<'_, '_, T>) {
            Inspector::<T>::step_end(&mut self.recorder, interp);
        }

        fn call(
            &mut self,
            interp: &mut Interpreter<'_, '_, T>,
            message: &mut Message<T>,
        ) -> Option<MessageResult<T>> {
            Inspector::<T>::call(&mut self.recorder, interp, message)
        }

        fn call_end(
            &mut self,
            interp: &mut Interpreter<'_, '_, T>,
            message: &Message<T>,
            result: &mut MessageResult<T>,
        ) {
            Inspector::<T>::call_end(&mut self.recorder, interp, message, result);
        }
    }

    #[test]
    fn memory_writes_reconstruct_interpreter_state_in_both_directions() {
        let target = Address::with_last_byte(0x40);
        let child = Address::with_last_byte(0x20);
        let mut db = InMemoryDB::default();
        // Stores, overlapping MCOPY, the three copy instructions, and expansion by MLOAD.
        // The child returns 32 bytes into a 64-byte output range; the rest must stay unchanged.
        let parent = hex::decode(concat!(
            "61abcd5f52",                 // MSTORE(0, 0xabcd).
            "6089600753",                 // MSTORE8(7, 0x89).
            "601f5f60085e",               // MCOPY(8, 0, 31).
            "601f60085f5e",               // MCOPY(0, 8, 31).
            "60295f608037",               // CALLDATACOPY(128, 0, 41).
            "60295f61010039",             // CODECOPY(256, 0, 41).
            "60295f61018060203c",         // EXTCODECOPY(child, 384, 0, 41).
            "6102005150",                 // MLOAD(512).
            "604261030052",               // MSTORE(768, 66).
            "60406102e05f5f5f60205af150", // CALL with output at 736.
            "3d5f6104003e",               // RETURNDATACOPY(1024, 0, RETURNDATASIZE).
            "60206104405f5f60205afa50",   // STATICCALL.
            "60206104605f5f60205af450",   // DELEGATECALL.
            "60206104805f5f5f60205af250", // CALLCODE.
            "00"
        ))
        .unwrap();
        db.insert_account_info(
            &target,
            AccountInfo::default().with_code(Bytecode::new_legacy(parent.into())),
        );
        db.insert_account_info(
            &child,
            AccountInfo::default()
                .with_code(Bytecode::new_legacy(hex::decode("61cafe5f5260205ff3").unwrap().into())),
        );
        let factory = EvmFactoryFor::<EthEvmNetwork>::default();
        let spec = SpecFor::<EthEvmNetwork>::default();
        let mut evm =
            factory.create_evm(db, EvmEnv::new(spec, factory.version(spec, 1), Default::default()));
        evm.set_inspector(Snapshots::default());
        let request = TransactionRequestFor::<EthEvmNetwork>::default()
            .with_from(Address::with_last_byte(0x30))
            .with_to(target)
            .with_nonce(0)
            .with_chain_id(1)
            .with_gas_limit(1_000_000)
            .with_max_fee_per_gas(0)
            .with_max_priority_fee_per_gas(0);
        let tx = EthEvmNetwork::tx_from_request(request).unwrap();
        let result = evm.transact(&tx).unwrap().commit();
        let Snapshots { recorder, steps } = *evm.clear_inspector_as::<Snapshots>().unwrap();
        assert!(result.status);
        let table = recorder.table.as_ref().unwrap();
        assert_eq!(table.len(), steps.len());
        for index in (0..table.len()).rev().chain(0..table.len()) {
            let step = table.step(index);
            assert_eq!(
                step.memory().as_deref(),
                Some(steps[index].0.as_slice()),
                "memory at {index}"
            );
            assert_eq!(step.stack().to_vec(), steps[index].1, "stack at {index}");
        }
    }
}
