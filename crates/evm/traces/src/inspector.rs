use crate::{
    CallTrace, CallTraceArena, CallTraceStep, LogCallOrder, TraceCallData, TraceLog, TraceRetData,
};
use alloy_primitives::{Address, Bytes, Log as RawLog, B256, U256};
use foundry_evm_core::{
    debug::Instruction::OpCode,
    utils::{gas_used, get_create_address, CallKind},
};
use revm::{
    interpreter::{
        opcode, return_ok, CallInputs, CallScheme, CreateInputs, InstructionResult, Interpreter,
        InterpreterResult,
    },
    Database, EvmContext, Inspector, JournalEntry,
};
use std::ops::Range;

/// An inspector that collects call traces.
#[derive(Default, Debug, Clone)]
pub struct Tracer {
    pub traces: CallTraceArena,
    trace_stack: Vec<usize>,
    step_stack: Vec<(usize, usize)>, // (trace_idx, step_idx)
    record_steps: bool,
}

impl Tracer {
    /// Enables step recording.
    pub fn record_steps(&mut self) {
        self.record_steps = true;
    }

    fn start_trace(
        &mut self,
        depth: usize,
        address: Address,
        data: Vec<u8>,
        value: U256,
        kind: CallKind,
        caller: Address,
    ) {
        self.trace_stack.push(self.traces.push_trace(
            0,
            CallTrace {
                depth,
                address,
                kind,
                data: TraceCallData::Raw(data.into()),
                value,
                status: InstructionResult::Continue,
                caller,
                ..Default::default()
            },
        ));
    }

    fn fill_trace(
        &mut self,
        status: InstructionResult,
        cost: u64,
        output: Bytes,
        address: Option<Address>,
    ) {
        let success = matches!(status, return_ok!());
        let trace = &mut self.traces.arena
            [self.trace_stack.pop().expect("more traces were filled than started")]
        .trace;
        trace.status = status;
        trace.success = success;
        trace.gas_cost = cost;
        trace.output = TraceRetData::Raw(output);

        if let Some(address) = address {
            trace.address = address;
        }
    }

    fn start_step<DB: Database>(&mut self, interp: &Interpreter, data: &EvmContext<'_, DB>) {
        let trace_idx =
            *self.trace_stack.last().expect("can't start step without starting a trace first");
        let node = &mut self.traces.arena[trace_idx];

        self.step_stack.push((trace_idx, node.trace.steps.len()));

        node.trace.steps.push(CallTraceStep {
            depth: data.journaled_state.depth(),
            pc: interp.program_counter(),
            op: OpCode(interp.current_opcode()),
            contract: interp.contract.address,
            stack: interp.stack.clone(),
            memory: interp.shared_memory.context_memory().to_vec(),
            gas: interp.gas.remaining(),
            gas_refund_counter: interp.gas.refunded() as u64,
            gas_cost: 0,
            state_diff: None,
            error: None,
        });
    }

    fn fill_step<DB: Database>(&mut self, interp: &Interpreter, data: &EvmContext<'_, DB>) {
        let (trace_idx, step_idx) =
            self.step_stack.pop().expect("can't fill step without starting a step first");
        let step = &mut self.traces.arena[trace_idx].trace.steps[step_idx];

        let op = interp.current_opcode();
        let journal_entry = data
            .journaled_state
            .journal
            .last()
            // This should always work because revm initializes it as `vec![vec![]]`
            .unwrap()
            .last();
        step.state_diff = match (op, journal_entry) {
            (
                opcode::SLOAD | opcode::SSTORE,
                Some(JournalEntry::StorageChange { address, key, .. }),
            ) => {
                let value = data.journaled_state.state[address].storage[key].present_value();
                Some((*key, value))
            }
            _ => None,
        };

        step.gas_cost = step.gas - interp.gas.remaining();

        // Error codes only
        if interp.instruction_result.is_error() {
            step.error = Some(format!("{:?}", interp.instruction_result));
        }
    }
}

impl<DB: Database> Inspector<DB> for Tracer {
    #[inline]
    fn step(&mut self, interp: &mut Interpreter, data: &mut EvmContext<'_, DB>) {
        if self.record_steps {
            self.start_step(interp, data);
        }
    }

    #[inline]
    fn step_end(&mut self, interp: &mut Interpreter, data: &mut EvmContext<'_, DB>) {
        if self.record_steps {
            self.fill_step(interp, data);
        }
    }

    #[inline]
    fn log(&mut self, _: &mut EvmContext<'_, DB>, _: &Address, topics: &[B256], data: &Bytes) {
        let node = &mut self.traces.arena[*self.trace_stack.last().expect("no ongoing trace")];
        node.ordering.push(LogCallOrder::Log(node.logs.len()));
        let data = data.clone();
        node.logs
            .push(TraceLog::Raw(RawLog::new(topics.to_vec(), data).expect("Received invalid log")));
    }

    #[inline]
    fn call(
        &mut self,
        data: &mut EvmContext<'_, DB>,
        inputs: &mut CallInputs,
    ) -> Option<(InterpreterResult, Range<usize>)> {
        let (from, to) = match inputs.context.scheme {
            CallScheme::DelegateCall | CallScheme::CallCode => {
                (inputs.context.address, inputs.context.code_address)
            }
            _ => (inputs.context.caller, inputs.context.address),
        };

        self.start_trace(
            data.journaled_state.depth() as usize,
            to,
            inputs.input.to_vec(),
            inputs.transfer.value,
            inputs.context.scheme.into(),
            from,
        );

        None
    }

    #[inline]
    fn call_end(
        &mut self,
        data: &mut EvmContext<'_, DB>,
        status: InterpreterResult,
    ) -> InterpreterResult {
        self.fill_trace(
            status.result,
            gas_used(data.env.cfg.spec_id, status.gas.spend(), status.gas.refunded() as u64),
            status.output.clone(),
            None,
        );

        status
    }

    #[inline]
    fn create(
        &mut self,
        data: &mut EvmContext<'_, DB>,
        inputs: &mut CreateInputs,
    ) -> Option<(InterpreterResult, Option<Address>)> {
        let _ = data.journaled_state.load_account(inputs.caller, data.db);
        let nonce = data.journaled_state.account(inputs.caller).info.nonce;
        self.start_trace(
            data.journaled_state.depth() as usize,
            get_create_address(inputs, nonce),
            inputs.init_code.to_vec(),
            inputs.value,
            inputs.scheme.into(),
            inputs.caller,
        );

        None
    }

    #[inline]
    fn create_end(
        &mut self,
        data: &mut EvmContext<'_, DB>,
        status: InterpreterResult,
        address: Option<Address>,
    ) -> (InterpreterResult, Option<Address>) {
        let code = address
            .and_then(|address| data.journaled_state.account(address).info.code.as_ref())
            .map(|code| code.original_bytes())
            .unwrap_or_default();
        self.fill_trace(
            status.result,
            gas_used(data.env.cfg.spec_id, status.gas.spend(), status.gas.refunded() as u64),
            code,
            address,
        );

        (status, address)
    }
}
