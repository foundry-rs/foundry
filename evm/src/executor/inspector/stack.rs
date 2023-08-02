use super::{Cheatcodes, ChiselState, Debugger, Fuzzer, LogCollector, TracePrinter, Tracer};
use crate::{
    coverage::HitMaps,
    debug::DebugArena,
    executor::{backend::DatabaseExt, inspector::CoverageCollector},
    trace::CallTraceArena,
};
use bytes::Bytes;
use ethers::{
    signers::LocalWallet,
    types::{Address, Log},
};
use revm::{
    inspectors::GasInspector,
    interpreter::{
        return_revert, CallInputs, CreateInputs, Gas, InstructionResult, Interpreter, Memory, Stack,
    },
    primitives::{B160, B256},
    EVMData, Inspector,
};
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

/// Helper macro to call the same method on multiple inspectors without resorting to dynamic
/// dispatch
#[macro_export]
macro_rules! call_inspectors {
    ($id:ident, [ $($inspector:expr),+ ], $call:block) => {
        $({
            if let Some($id) = $inspector {
                $call;
            }
        })+
    }
}

pub struct InspectorData {
    pub logs: Vec<Log>,
    pub labels: BTreeMap<Address, String>,
    pub traces: Option<CallTraceArena>,
    pub debug: Option<DebugArena>,
    pub coverage: Option<HitMaps>,
    pub gas: Option<u64>,
    pub cheatcodes: Option<Cheatcodes>,
    pub script_wallets: Vec<LocalWallet>,
    pub chisel_state: Option<(Stack, Memory, InstructionResult)>,
    pub raw_exported_data: crate::executor::RawExportedData,
}

/// An inspector that calls multiple inspectors in sequence.
///
/// If a call to an inspector returns a value other than [InstructionResult::Continue] (or
/// equivalent) the remaining inspectors are not called.
#[derive(Default)]
pub struct InspectorStack {
    pub tracer: Option<Tracer>,
    pub logs: Option<LogCollector>,
    pub cheatcodes: Option<Cheatcodes>,
    pub gas: Option<Rc<RefCell<GasInspector>>>,
    pub debugger: Option<Debugger>,
    pub fuzzer: Option<Fuzzer>,
    pub coverage: Option<CoverageCollector>,
    pub printer: Option<TracePrinter>,
    pub chisel_state: Option<ChiselState>,
}

impl InspectorStack {
    pub fn collect_inspector_states(self) -> InspectorData {
        InspectorData {
            logs: self.logs.map(|logs| logs.logs).unwrap_or_default(),
            labels: self
                .cheatcodes
                .as_ref()
                .map(|cheatcodes| cheatcodes.labels.clone())
                .unwrap_or_default(),
            traces: self.tracer.map(|tracer| tracer.traces),
            debug: self.debugger.map(|debugger| debugger.arena),
            coverage: self.coverage.map(|coverage| coverage.maps),
            gas: self.gas.map(|gas| gas.borrow().gas_remaining()),
            script_wallets: self
                .cheatcodes
                .as_ref()
                .map(|cheatcodes| cheatcodes.script_wallets.clone())
                .unwrap_or_default(),
            raw_exported_data: self
                .cheatcodes
                .as_ref()
                .map(|cheatcodes| cheatcodes.raw_exported_data.clone())
                .unwrap_or_default(),
            cheatcodes: self.cheatcodes,
            chisel_state: self.chisel_state.unwrap_or_default().state,
        }
    }

    fn do_call_end<DB: DatabaseExt>(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
        is_static: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let (new_status, new_gas, new_retdata) = inspector.call_end(
                    data,
                    call,
                    remaining_gas,
                    status,
                    retdata.clone(),
                    is_static,
                );

                // If the inspector returns a different status or a revert with a non-empty message,
                // we assume it wants to tell us something
                if new_status != status ||
                    (new_status == InstructionResult::Revert && new_retdata != retdata)
                {
                    return (new_status, new_gas, new_retdata)
                }
            }
        );

        (status, remaining_gas, retdata)
    }
}

impl<DB> Inspector<DB> for InspectorStack
where
    DB: DatabaseExt,
{
    fn initialize_interp(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.debugger,
                &mut self.coverage,
                &mut self.tracer,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let status = inspector.initialize_interp(interpreter, data, is_static);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    return status
                }
            }
        );

        InstructionResult::Continue
    }

    fn step(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let status = inspector.step(interpreter, data, is_static);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    return status
                }
            }
        );

        InstructionResult::Continue
    }

    fn log(
        &mut self,
        evm_data: &mut EVMData<'_, DB>,
        address: &B160,
        topics: &[B256],
        data: &Bytes,
    ) {
        call_inspectors!(
            inspector,
            [&mut self.tracer, &mut self.logs, &mut self.cheatcodes, &mut self.printer],
            {
                inspector.log(evm_data, address, topics, data);
            }
        );
    }

    fn step_end(
        &mut self,
        interpreter: &mut Interpreter,
        data: &mut EVMData<'_, DB>,
        is_static: bool,
        status: InstructionResult,
    ) -> InstructionResult {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.debugger,
                &mut self.tracer,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer,
                &mut self.chisel_state
            ],
            {
                let status = inspector.step_end(interpreter, data, is_static, status);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    return status
                }
            }
        );

        InstructionResult::Continue
    }

    fn call(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CallInputs,
        is_static: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.fuzzer,
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let (status, gas, retdata) = inspector.call(data, call, is_static);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    return (status, gas, retdata)
                }
            }
        );

        (InstructionResult::Continue, Gas::new(call.gas_limit), Bytes::new())
    }

    fn call_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CallInputs,
        remaining_gas: Gas,
        status: InstructionResult,
        retdata: Bytes,
        is_static: bool,
    ) -> (InstructionResult, Gas, Bytes) {
        let res = self.do_call_end(data, call, remaining_gas, status, retdata, is_static);

        if matches!(res.0, return_revert!()) {
            // Encountered a revert, since cheatcodes may have altered the evm state in such a way
            // that violates some constraints, e.g. `deal`, we need to manually roll back on revert
            // before revm reverts the state itself
            if let Some(cheats) = self.cheatcodes.as_mut() {
                cheats.on_revert(data);
            }
        }

        res
    }

    fn create(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &mut CreateInputs,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let (status, addr, gas, retdata) = inspector.create(data, call);

                // Allow inspectors to exit early
                if status != InstructionResult::Continue {
                    return (status, addr, gas, retdata)
                }
            }
        );

        (InstructionResult::Continue, None, Gas::new(call.gas_limit), Bytes::new())
    }

    fn create_end(
        &mut self,
        data: &mut EVMData<'_, DB>,
        call: &CreateInputs,
        status: InstructionResult,
        address: Option<B160>,
        remaining_gas: Gas,
        retdata: Bytes,
    ) -> (InstructionResult, Option<B160>, Gas, Bytes) {
        call_inspectors!(
            inspector,
            [
                &mut self.gas.as_deref().map(|gas| gas.borrow_mut()),
                &mut self.debugger,
                &mut self.tracer,
                &mut self.coverage,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer
            ],
            {
                let (new_status, new_address, new_gas, new_retdata) = inspector.create_end(
                    data,
                    call,
                    status,
                    address,
                    remaining_gas,
                    retdata.clone(),
                );

                if new_status != status {
                    return (new_status, new_address, new_gas, new_retdata)
                }
            }
        );

        (status, address, remaining_gas, retdata)
    }

    fn selfdestruct(&mut self, contract: B160, target: B160) {
        call_inspectors!(
            inspector,
            [
                &mut self.debugger,
                &mut self.tracer,
                &mut self.logs,
                &mut self.cheatcodes,
                &mut self.printer,
                &mut self.chisel_state
            ],
            {
                Inspector::<DB>::selfdestruct(inspector, contract, target);
            }
        );
    }
}
