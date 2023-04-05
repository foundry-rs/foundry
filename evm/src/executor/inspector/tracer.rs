use crate::{
    debug::Instruction::OpCode,
    executor::inspector::utils::{gas_used, get_create_address},
    trace::{
        CallTrace, CallTraceArena, CallTraceStep, LogCallOrder, RawOrDecodedCall, RawOrDecodedLog,
        RawOrDecodedReturnData,
    },
    CallKind,
};
use bytes::Bytes;
use ethers::{
    abi::RawLog,
    types::{Address, H256, U256},
};
use revm::{Database, EVMData, Inspector, JournalEntry, primitives::{B160, B256}};
use revm::inspectors::GasInspector;
use revm::interpreter::{CallInputs, CallScheme, CreateInputs, Gas, InstructionResult, Interpreter, opcode, return_ok};
use std::{cell::RefCell, rc::Rc};

/// An inspector that collects call traces.
#[derive(Default, Debug, Clone)]
pub struct Tracer {
    record_steps: bool,

    pub traces: CallTraceArena,
    trace_stack: Vec<usize>,
    step_stack: Vec<(usize, usize)>, // (trace_idx, step_idx)

    gas_inspector: Rc<RefCell<GasInspector>>,
}

impl Tracer {
    /// Enables step recording and uses [revm::GasInspector] to report gas costs for each step.
    ///
    /// Gas Inspector should be called externally **before** [Tracer], this is why we need it as
    /// `Rc<RefCell<_>>` here.
    pub fn with_steps_recording(mut self, gas_inspector: Rc<RefCell<GasInspector>>) -> Self {
        self.record_steps = true;
        self.gas_inspector = gas_inspector;
        self
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
                data: RawOrDecodedCall::Raw(data.into()),
                value,
                status: Return::Continue,
                caller,
                ..Default::default()
            },
        ));
    }

    fn fill_trace(&mut self, status: Return, cost: u64, output: Vec<u8>, address: Option<Address>) {
        let success = matches!(status, return_ok!());
        let trace = &mut self.traces.arena
            [self.trace_stack.pop().expect("more traces were filled than started")]
        .trace;
        trace.status = status;
        trace.success = success;
        trace.gas_cost = cost;
        trace.output = RawOrDecodedReturnData::Raw(output.into());

        if let Some(address) = address {
            trace.address = address;
        }
    }

    fn start_step<DB: Database>(&mut self, interp: &mut Interpreter, data: &mut EVMData<'_, DB>) {
        let trace_idx =
            *self.trace_stack.last().expect("can't start step without starting a trace first");
        let trace = &mut self.traces.arena[trace_idx];

        self.step_stack.push((trace_idx, trace.trace.steps.len()));

        let pc = interp.program_counter();

        trace.trace.steps.push(CallTraceStep {
            depth: data.journaled_state.depth(),
            pc,
            op: OpCode(interp.contract.bytecode.bytecode()[pc]),
            contract: H256::from_slice(interp.contract.address.as_bytes()).into(),
            stack: interp.stack.clone(),
            memory: interp.memory.clone(),
            gas: self.gas_inspector.borrow().gas_remaining(),
            gas_refund_counter: interp.gas.refunded() as u64,
            gas_cost: 0,
            state_diff: None,
            error: None,
        });
    }

    fn fill_step<DB: Database>(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        status: Return,
    ) {
        let (trace_idx, step_idx) =
            self.step_stack.pop().expect("can't fill step without starting a step first");
        let step = &mut self.traces.arena[trace_idx].trace.steps[step_idx];

        if let Some(pc) = interp.program_counter().checked_sub(1) {
            let op = interp.contract.bytecode.bytecode()[pc];

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

            step.gas_cost = step.gas - self.gas_inspector.borrow().gas_remaining();
        }

        // Error codes only
        if status as u8 > Return::OutOfGas as u8 {
            step.error = Some(format!("{status:?}"));
        }
    }
}

impl<DB> Inspector<DB> for Tracer
where
    DB: Database,
{
    fn step(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _is_static: bool,
    ) -> Return {
        if !self.record_steps {
            return Return::Continue
        }

        self.start_step(interp, data);

        Return::Continue
    }

    fn log(&mut self, _: &mut EVMData<'_, DB>, _: &B160, topics: &[B256], data: &Bytes) {
        let node = &mut self.traces.arena[*self.trace_stack.last().expect("no ongoing trace")];
        let topics: Vec<_> = topics.to_vec().into_iter().map(|t| H256::from_slice(t.as_bytes())).collect();
        node.ordering.push(LogCallOrder::Log(node.logs.len()));
        node.logs
            .push(RawOrDecodedLog::Raw(RawLog { topics: topics, data: data.to_vec() }));
    }

    fn step_end(
        &mut self,
        interp: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        _is_static: bool,
        status: Return,
    ) -> Return {
        if !self.record_steps {
            return Return::Continue
        }

        self.fill_step(interp, data, status);

        status
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &mut CallInputs,
        _is_static: bool,
    ) -> (Return, Gas, Bytes) {
        let (from, to) = match inputs.context.scheme {
            CallScheme::DelegateCall | CallScheme::CallCode => {
                (inputs.context.address, inputs.context.code_address)
            }
            _ => (inputs.context.caller, inputs.context.address),
        };

        self.start_trace(
            data.journaled_state.depth() as usize,
            Address::from_slice(to.as_bytes()).into(),
            inputs.input.to_vec(),
            inputs.transfer.value,
            inputs.context.scheme.into(),
            Address::from_slice(from.as_bytes()).into(),
        );

        (Return::Continue, Gas::new(inputs.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _inputs: &CallInputs,
        gas: Gas,
        status: Return,
        retdata: Bytes,
        _is_static: bool,
    ) -> (Return, Gas, Bytes) {
        self.fill_trace(
            status,
            gas_used(data.env.cfg.spec_id, gas.spend(), gas.refunded() as u64),
            retdata.to_vec(),
            None,
        );

        (status, gas, retdata)
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        inputs: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        // TODO: Does this increase gas cost?
        let _ = data.journaled_state.load_account(inputs.caller, data.db);
        let nonce = data.journaled_state.account(inputs.caller).info.nonce;
        self.start_trace(
            data.journaled_state.depth() as usize,
            get_create_address(inputs, nonce),
            inputs.init_code.to_vec(),
            inputs.value,
            inputs.scheme.into(),
            Address::from_slice(inputs.caller.as_bytes()).into(),
        );

        (Return::Continue, None, Gas::new(inputs.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        _inputs: &CreateInputs,
        status: InstructionResult,
        address: Option<B160>,
        gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        let code = match address {
            Some(address) => data
                .journaled_state
                .account(address)
                .info
                .code
                .as_ref()
                .map_or(vec![], |code| code.bytes()[..code.len()].to_vec()),
            None => vec![],
        };
        self.fill_trace(
            status,
            gas_used(data.env.cfg.spec_id, gas.spend(), gas.refunded() as u64),
            code,
            address.map(|addr| Address::from_slice(addr.as_bytes()).into()),
        );

        (status, address, gas, retdata)
    }
}
