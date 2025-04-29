//! Anvil specific [`revm::Inspector`] implementation

use crate::{eth::macros::node_info, foundry_common::ErrorExt};
use alloy_evm::eth::EthEvmContext;
use alloy_primitives::{Address, Log, U256};
use alloy_sol_types::SolInterface;
use foundry_evm::{
    backend::DatabaseError,
    call_inspectors,
    constants::HARDHAT_CONSOLE_ADDRESS,
    decode::decode_console_logs,
    inspectors::{hh_to_ds, TracingInspector},
    traces::{
        render_trace_arena_inner, CallTraceDecoder, SparsedTraceArena, TracingInspectorConfig,
    },
};
use foundry_evm_core::abi::console;
use revm::{
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{
        interpreter::EthInterpreter, CallInputs, CallOutcome, CreateInputs, CreateOutcome,
        EOFCreateInputs, Gas, InstructionResult, Interpreter, InterpreterResult,
    },
    Database, Inspector,
};

/// The [`revm::Inspector`] used when transacting in the evm
#[derive(Clone, Debug, Default)]
pub struct AnvilInspector {
    /// Collects all traces
    pub tracer: Option<TracingInspector>,
    /// Collects all `console.sol` logs
    pub log_collector: Option<LogCollector>,
}

impl AnvilInspector {
    /// Called after the inspecting the evm
    ///
    /// This will log all `console.sol` logs
    pub fn print_logs(&self) {
        if let Some(collector) = &self.log_collector {
            print_logs(&collector.logs);
        }
    }

    /// Consumes the type and prints the traces.
    pub fn into_print_traces(mut self) {
        if let Some(a) = self.tracer.take() {
            print_traces(a)
        }
    }

    /// Called after the inspecting the evm
    /// This will log all traces
    pub fn print_traces(&self) {
        if let Some(a) = self.tracer.clone() {
            print_traces(a)
        }
    }

    /// Configures the `Tracer` [`revm::Inspector`]
    pub fn with_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().set_steps(false)));
        self
    }

    pub fn with_tracing_config(mut self, config: TracingInspectorConfig) -> Self {
        self.tracer = Some(TracingInspector::new(config));
        self
    }

    /// Enables steps recording for `Tracer`.
    pub fn with_steps_tracing(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().with_state_diffs()));
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a log collector
    pub fn with_log_collector(mut self) -> Self {
        self.log_collector = Some(Default::default());
        self
    }

    /// Configures the `Tracer` [`revm::Inspector`] with a trace printer
    pub fn with_trace_printer(mut self) -> Self {
        self.tracer = Some(TracingInspector::new(TracingInspectorConfig::all().with_state_diffs()));
        self
    }
}

/// Prints the traces for the inspector
///
/// Caution: This blocks on call trace decoding
///
/// # Panics
///
/// If called outside tokio runtime
fn print_traces(tracer: TracingInspector) {
    let arena = tokio::task::block_in_place(move || {
        tokio::runtime::Handle::current().block_on(async move {
            let mut arena = tracer.into_traces();
            let decoder = CallTraceDecoder::new();
            decoder.populate_traces(arena.nodes_mut()).await;
            arena
        })
    });

    let traces = SparsedTraceArena { arena, ignored: Default::default() };
    node_info!("Traces:");
    node_info!("{}", render_trace_arena_inner(&traces, false, true));
}

impl<CTX, D> Inspector<CTX, EthInterpreter> for AnvilInspector
where
    D: Database<Error = DatabaseError>,
    CTX: ContextTr<Db = D>,
    CTX::Journal: JournalExt,
{
    fn initialize_interp(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.initialize_interp(interp, ecx);
        });
    }

    fn step(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step(interp, ecx);
        });
    }

    fn step_end(&mut self, interp: &mut Interpreter, ecx: &mut CTX) {
        call_inspectors!([&mut self.tracer], |inspector| {
            inspector.step_end(interp, ecx);
        });
    }

    fn log(&mut self, interp: &mut Interpreter, ecx: &mut CTX, log: Log) {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| {
            // TODO: rm the log.clone
            inspector.log(interp, ecx, log.clone());
        });
    }

    fn call(&mut self, ecx: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        call_inspectors!([&mut self.tracer, &mut self.log_collector], |inspector| inspector
            .call(ecx, inputs)
            .map(Some),);
        None
    }

    fn call_end(&mut self, ecx: &mut CTX, inputs: &CallInputs, outcome: &mut CallOutcome) {
        if let Some(tracer) = &mut self.tracer {
            tracer.call_end(ecx, inputs, outcome);
        }
    }

    fn create(&mut self, ecx: &mut CTX, inputs: &mut CreateInputs) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.create(ecx, inputs) {
                return Some(out);
            }
        }
        None
    }

    fn create_end(&mut self, ecx: &mut CTX, inputs: &CreateInputs, outcome: &mut CreateOutcome) {
        if let Some(tracer) = &mut self.tracer {
            tracer.create_end(ecx, inputs, outcome);
        }
    }

    #[inline]
    fn eofcreate(&mut self, ecx: &mut CTX, inputs: &mut EOFCreateInputs) -> Option<CreateOutcome> {
        if let Some(tracer) = &mut self.tracer {
            if let Some(out) = tracer.eofcreate(ecx, inputs) {
                return Some(out);
            }
        }
        None
    }

    #[inline]
    fn eofcreate_end(
        &mut self,
        ecx: &mut CTX,
        inputs: &EOFCreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if let Some(tracer) = &mut self.tracer {
            tracer.eofcreate_end(ecx, inputs, outcome);
        }
    }

    #[inline]
    fn selfdestruct(&mut self, contract: Address, target: Address, value: U256) {
        if let Some(tracer) = &mut self.tracer {
            Inspector::<EthEvmContext<D>>::selfdestruct(tracer, contract, target, value);
        }
    }
}

/// Prints all the logs
pub fn print_logs(logs: &[Log]) {
    for log in decode_console_logs(logs) {
        tracing::info!(target: crate::logging::EVM_CONSOLE_LOG_TARGET, "{}", log);
    }
}

// DUPLICATION foundry_evm
// Duplicated workaround due to the `FoundryEvmContext` database being hardcoded to `dyn
// DatabaseExt` instead of being generic.
/// An inspector that collects logs during execution.
///
/// The inspector collects logs from the `LOG` opcodes as well as Hardhat-style `console.sol` logs.
#[derive(Clone, Debug, Default)]
pub struct LogCollector {
    /// The collected logs. Includes both `LOG` opcodes and Hardhat-style `console.sol` logs.
    pub logs: Vec<Log>,
}

impl LogCollector {
    #[cold]
    fn do_hardhat_log(&mut self, inputs: &CallInputs) -> Option<CallOutcome> {
        if let Err(err) = self.hardhat_log(&inputs.input) {
            let result = InstructionResult::Revert;
            let output = err.abi_encode_revert();
            return Some(CallOutcome {
                result: InterpreterResult { result, output, gas: Gas::new(inputs.gas_limit) },
                memory_offset: inputs.return_memory_offset.clone(),
            })
        }
        None
    }

    fn hardhat_log(&mut self, data: &[u8]) -> alloy_sol_types::Result<()> {
        let decoded = console::hh::ConsoleCalls::abi_decode(data, false)?;
        self.logs.push(hh_to_ds(&decoded));
        Ok(())
    }
}

impl<CTX, DB: Database<Error = DatabaseError>> Inspector<CTX, EthInterpreter> for LogCollector
where
    CTX: ContextTr<Db = DB>,
    CTX::Journal: JournalExt,
{
    fn log(&mut self, _interp: &mut Interpreter, _context: &mut CTX, log: Log) {
        self.logs.push(log);
    }

    fn call(&mut self, _context: &mut CTX, inputs: &mut CallInputs) -> Option<CallOutcome> {
        if inputs.target_address == HARDHAT_CONSOLE_ADDRESS {
            return self.do_hardhat_log(inputs);
        }
        None
    }
}
