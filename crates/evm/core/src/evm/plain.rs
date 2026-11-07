//! A mainnet handler that runs frames on the interpreter's plain loop when the inspector needs no
//! callback for each instruction.
//!
//! revm's inspected loop calls [`Inspector::step`] and [`Inspector::step_end`] around every
//! instruction and checks the journal for new logs after each one, even when every inspector's
//! `step` does nothing. Foundry's inspector stack often needs no per-instruction callback, such as
//! in a fuzz campaign whose coverage and comparison operands come from compiler instrumentation
//! (see [`edgecov`](crate::edgecov)). Then [`InspectorExt::run_frame_plain`] lets the frame run on
//! [`Interpreter::run_plain`], the loop of an uninspected EVM, while frame, call, create, log and
//! selfdestruct hooks run as in revm's inspected loop.

use crate::InspectorExt;
use revm::{
    context::{ContextTr, JournalEntry, JournalTr, result::HaltReason},
    handler::{
        EthFrame, EvmTr, EvmTrError, FrameResult, Handler, ItemOrResult,
        evm::{ContextDbError, FrameTr},
        instructions::InstructionProvider,
    },
    inspector::{Inspector, InspectorEvmTr, InspectorHandler, JournalExt, handler::frame_end},
    interpreter::{
        Host, InstructionResult, Interpreter, InterpreterAction,
        instructions::{GasTable, InstructionTable},
        interpreter::EthInterpreter,
        interpreter_action::FrameInit,
        interpreter_types::LoopControl,
    },
    state::EvmState,
};
use std::marker::PhantomData;

/// Ethereum's mainnet handler, whose frames run on the interpreter's plain loop whenever the
/// inspector allows it ([`InspectorExt::run_frame_plain`]).
#[derive(Debug)]
pub struct PlainLoopHandler<EVM, ERROR, FRAME> {
    _phantom: PhantomData<(EVM, ERROR, FRAME)>,
}

impl<EVM, ERROR, FRAME> Default for PlainLoopHandler<EVM, ERROR, FRAME> {
    fn default() -> Self {
        Self { _phantom: PhantomData }
    }
}

impl<EVM, ERROR, FRAME> Handler for PlainLoopHandler<EVM, ERROR, FRAME>
where
    EVM: EvmTr<Context: ContextTr<Journal: JournalTr<State = EvmState>>, Frame = FRAME>,
    ERROR: EvmTrError<EVM>,
    FRAME: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = HaltReason;
}

impl<EVM, ERROR> InspectorHandler for PlainLoopHandler<EVM, ERROR, EthFrame<EthInterpreter>>
where
    EVM: InspectorEvmTr<
            Context: ContextTr<Journal: JournalTr<State = EvmState>>,
            Frame = EthFrame<EthInterpreter>,
            Inspector: Inspector<<EVM as EvmTr>::Context, EthInterpreter> + InspectorExt,
        >,
    ERROR: EvmTrError<EVM>,
{
    type IT = EthInterpreter;

    /// revm's inspected execution loop, with [`run_frame`] in place of
    /// [`InspectorEvmTr::inspect_frame_run`].
    fn inspect_run_exec_loop(
        &mut self,
        evm: &mut Self::Evm,
        first_frame_input: FrameInit,
    ) -> Result<FrameResult, Self::Error> {
        if let ItemOrResult::Result(frame_result) = evm.inspect_frame_init(first_frame_input)? {
            return Ok(frame_result);
        }

        loop {
            let result = match run_frame(evm)? {
                ItemOrResult::Item(init) => match evm.inspect_frame_init(init)? {
                    ItemOrResult::Item(_) => continue,
                    // No frame was pushed, so none is popped.
                    ItemOrResult::Result(result) => result,
                },
                ItemOrResult::Result(result) => result,
            };

            if let Some(result) = evm.frame_return_result(result)? {
                return Ok(result);
            }
        }
    }
}

/// How many instructions a frame runs on the plain loop between calls to
/// [`InspectorExt::poll_plain_frame`].
pub const PLAIN_POLL_INTERVAL: usize = 1024;

/// [`Interpreter::run_plain`], polling the inspector every [`PLAIN_POLL_INTERVAL`] instructions
/// so that a long-running frame can still be cancelled.
fn run_polled<H: Host + ?Sized>(
    interpreter: &mut Interpreter,
    instruction_table: &InstructionTable<EthInterpreter, H>,
    gas_table: &GasTable,
    host: &mut H,
    inspector: &mut impl InspectorExt,
) -> InterpreterAction {
    let error = 'run: loop {
        for _ in 0..PLAIN_POLL_INTERVAL {
            if let Err(error) = interpreter.step(instruction_table, gas_table, host) {
                break 'run Some(error);
            }
        }
        inspector.poll_plain_frame(interpreter);
        if interpreter.bytecode.is_end() {
            break None;
        }
    };
    if let Some(error) = error
        && interpreter.bytecode.action.is_none()
    {
        interpreter.halt(error);
    }
    interpreter.take_next_action()
}

/// Runs the frame on top of the stack until it returns or calls, on the plain loop when the
/// inspector allows it, and otherwise as [`InspectorEvmTr::inspect_frame_run`] does.
fn run_frame<EVM>(
    evm: &mut EVM,
) -> Result<ItemOrResult<FrameInit, FrameResult>, ContextDbError<EVM::Context>>
where
    EVM: InspectorEvmTr<
            Frame = EthFrame<EthInterpreter>,
            Inspector: Inspector<<EVM as EvmTr>::Context, EthInterpreter> + InspectorExt,
        >,
{
    let (ctx, inspector, frame, instructions) = evm.ctx_inspector_frame_instructions();
    if !inspector.run_frame_plain(&mut frame.interpreter) {
        return evm.inspect_frame_run();
    }

    let interpreter = &mut frame.interpreter;
    let logs_i = ctx.journal().logs().len();
    let journal_i = ctx.journal().journal().len();
    let next_action = if interpreter.bytecode.is_end() {
        // The inspector halted the frame.
        interpreter.take_next_action()
    } else {
        run_polled(
            interpreter,
            instructions.instruction_table(),
            instructions.gas_table(),
            ctx,
            inspector,
        )
    };
    inspector.plain_frame_suspended(interpreter);

    // The inspected loop reports each log after the instruction that emitted it. Instructions run
    // here can only emit logs and suspend the frame, so the logs keep their order relative to the
    // calls and frames around them.
    if ctx.journal().logs().len() != logs_i {
        let logs = ctx.journal().logs()[logs_i..].to_vec();
        for log in logs {
            inspector.log_full(interpreter, ctx, log);
        }
    }

    if let InterpreterAction::Return(result) = &next_action
        && result.result == InstructionResult::SelfDestruct
        && let Some(
            JournalEntry::AccountDestroyed {
                address: contract,
                target: to,
                had_balance: balance,
                ..
            }
            | JournalEntry::BalanceTransfer { from: contract, to, balance, .. },
        ) = ctx.journal().journal().get(journal_i..).and_then(<[_]>::last)
    {
        let (contract, to, balance) = (*contract, *to, *balance);
        inspector.selfdestruct(contract, to, balance);
    }

    let mut result = frame.process_next_action(ctx, next_action);
    if let Ok(ItemOrResult::Result(frame_result)) = &mut result {
        frame_end(ctx, inspector, &frame.input, frame_result);
        frame.set_finished(true);
    }
    result
}
