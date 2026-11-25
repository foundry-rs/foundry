//! Internal call tracking from the JUMP and JUMPDEST steps of a call.
//!
//! solc lists every internal function it compiles in `functionDebugData`, with the offset of its
//! entry `JUMPDEST` and the number of stack words it takes and returns. A `JUMP` to such an
//! offset is a call. The caller pushed the return address below the arguments, so a later `JUMP`
//! to that address, at the stack height the return leaves, is the return. This holds in
//! optimized code too, where the source maps' jump markers and locations do not.
//!
//! Calls the optimizer inlined leave no jump and are not visible. A function without an entry,
//! such as an inline assembly function in legacy codegen, is found from the source map's jump
//! marker instead, and paired by return address as well.
//!
//! Functions that the compiler generated, such as ABI decoders and checked arithmetic, stay on
//! the frame stack but are not shown. The external function that the dispatcher enters is the
//! call that the trace shows already, so it is not shown either. Via-IR codegen calls modifiers
//! like functions, so they appear as calls there. Legacy codegen inlines them.

use super::{DecodedInternalCall, decode_parameters, functions::FunctionInfo};
use crate::CallTraceStep;
use alloy_primitives::U256;
use evm2::interpreter::opcode::OpCode;
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// A function that `functionDebugData` lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FunctionEntry {
    /// The name that the key of the entry spells, if it is a function of the contract rather than
    /// one the compiler generated.
    pub name: Option<String>,
    /// The stack words the function takes.
    pub parameter_slots: usize,
    /// The stack words the function leaves.
    pub return_slots: usize,
}

/// The entry points and jump destinations of one code of a contract, which creation code and
/// runtime code each have.
#[derive(Clone, Debug, Default)]
pub struct CodeFunctions {
    /// The function entries by offset of their first `JUMPDEST`. Functions that the compiler
    /// merged into one share an offset.
    pub entries: HashMap<u32, Vec<FunctionEntry>>,
    /// The offsets of all `JUMPDEST`s.
    pub jumpdests: HashSet<u32>,
}

/// How the compiler lays out the stack when it enters and leaves a function.
///
/// All layouts hold the return address below the arguments, so that an entry with `n` parameter
/// words has the return address `n` words below the stack top, and a return leaves its values
/// below the stack top, first value deepest. The layouts differ in the order of the arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallConvention {
    /// solc's legacy code generator: the first argument is the deepest.
    SolcLegacy,
    /// solc's via-IR code generator: the first argument is on top.
    SolcViaIr,
}

impl CallConvention {
    /// The convention of a solc artifact, from the `viaIR` setting in its metadata.
    pub fn solc(via_ir: Option<bool>) -> Option<Self> {
        via_ir.map(|via_ir| if via_ir { Self::SolcViaIr } else { Self::SolcLegacy })
    }

    /// The arguments in `stack` in parameter order, given the index of the return address.
    fn arguments(self, stack: &[U256], return_address_index: usize) -> Cow<'_, [U256]> {
        let arguments = &stack[return_address_index + 1..];
        match self {
            Self::SolcLegacy => Cow::Borrowed(arguments),
            Self::SolcViaIr => Cow::Owned(arguments.iter().rev().copied().collect()),
        }
    }
}

/// An internal call that the tracker found.
#[derive(Debug)]
pub struct InternalCall {
    /// The step of the `JUMP` into the function.
    pub start: usize,
    /// The step of the `JUMPDEST` that the function returned to, or the step before the jump that
    /// took over the frame of a function that does not return.
    pub end: usize,
    /// The function and its decoded arguments and return values.
    pub call: DecodedInternalCall,
}

/// The stack words that a function takes and returns, with the function.
pub type ResolvedEntry = (usize, usize, FunctionInfo);

/// A function that the source map found, with its stack layout.
pub struct SourceMapFunction {
    pub function: FunctionInfo,
    /// The stack words it takes and returns.
    pub parameter_slots: usize,
    pub return_slots: usize,
    /// The order of its arguments on the stack, if known.
    pub convention: Option<CallConvention>,
}

/// What the tracker knows about the contract whose steps it walks.
pub struct TrackerContext<'a> {
    /// The functions of the code that ran.
    pub code: &'a CodeFunctions,
    /// Names the function that a call to the entry at an offset runs, among the entries there.
    /// Returns `None` for an entry that is not shown.
    pub resolve: &'a dyn Fn(u32, &[FunctionEntry]) -> Option<ResolvedEntry>,
    /// Whether the code is creation code, which no dispatcher enters.
    pub is_create: bool,
    /// The stack layout of the code, if known. Without it the arguments are not decoded.
    pub convention: Option<CallConvention>,
    /// The call data of the call, for `calldata` parameters.
    pub calldata: &'a [u8],
    /// Whether the call data selects the external function with a declared name, which the
    /// dispatcher then enters.
    pub is_dispatched: &'a dyn Fn(&str) -> bool,
    /// Finds the function that the `JUMP` at a step enters from the source map, for a function
    /// that `functionDebugData` does not list.
    pub from_source_map: &'a dyn Fn(usize) -> Option<SourceMapFunction>,
}

/// A call that has not returned.
struct Frame {
    /// The step of the `JUMP` into the function.
    start: usize,
    /// The return address and its index in the stack.
    return_address: U256,
    return_address_index: usize,
    return_slots: usize,
    /// The function, if the call is shown.
    function: Option<Arc<FunctionInfo>>,
    /// The decoded arguments, if they are known.
    args: Option<Vec<String>>,
}

/// Finds the internal calls in the `JUMP` and `JUMPDEST` steps of one call frame.
pub fn track_internal_calls(
    steps: &[CallTraceStep],
    context: &TrackerContext<'_>,
) -> Vec<InternalCall> {
    let mut tracker = Tracker {
        steps,
        context,
        frames: Vec::new(),
        calls: Vec::new(),
        resolved: HashMap::new(),
        entered: false,
    };
    for (i, step) in steps.iter().enumerate() {
        if step.op == OpCode::JUMP {
            tracker.jump(i);
        }
    }
    tracker.finish()
}

struct Tracker<'a> {
    steps: &'a [CallTraceStep],
    context: &'a TrackerContext<'a>,
    frames: Vec<Frame>,
    calls: Vec<InternalCall>,
    /// The stack words and function of each entry offset that a call went to.
    resolved: HashMap<u32, (usize, usize, Option<Arc<FunctionInfo>>)>,
    /// Whether the external function that the call node runs has been entered.
    entered: bool,
}

impl Tracker<'_> {
    /// Handles the `JUMP` at step `i`: a call into a function, a return from one, or neither.
    fn jump(&mut self, i: usize) {
        let Some(landing) = self.steps.get(i + 1).filter(|step| step.op == OpCode::JUMPDEST) else {
            return;
        };
        let (Some(&target), Some(landed)) = (
            self.steps[i].stack.as_deref().and_then(|stack| stack.last()),
            landing.stack.as_deref(),
        ) else {
            return;
        };
        if U256::from(landing.pc) != target {
            return;
        }

        let context = self.context;
        if let Some(entries) = context.code.entries.get(&landing.pc) {
            let (parameter_slots, return_slots, function) = self
                .resolved
                .entry(landing.pc)
                .or_insert_with(|| match (context.resolve)(landing.pc, entries) {
                    Some((parameter_slots, return_slots, function)) => {
                        (parameter_slots, return_slots, Some(Arc::new(function)))
                    }
                    None => (entries[0].parameter_slots, entries[0].return_slots, None),
                })
                .clone();
            let convention = self.context.convention;
            self.call(i, parameter_slots, return_slots, function, convention);
        } else if let Some(k) = self.frames.iter().rposition(|frame| {
            frame.return_address == target
                && landed.len() == frame.return_address_index + frame.return_slots
        }) {
            self.ret(i, k, landing);
        } else if let Some(found) = (self.context.from_source_map)(i) {
            let function = Some(Arc::new(found.function));
            self.call(i, found.parameter_slots, found.return_slots, function, found.convention);
        }
    }

    /// Handles the `JUMP` at step `i` to the entry of a function that takes `parameter_slots` and
    /// returns `return_slots` stack words, which `function` names.
    fn call(
        &mut self,
        i: usize,
        parameter_slots: usize,
        return_slots: usize,
        mut function: Option<Arc<FunctionInfo>>,
        convention: Option<CallConvention>,
    ) {
        let entered = &self.steps[i + 1];
        let Some(stack) = entered.stack.as_deref() else { return };
        // A function without a return address never returns, like the functions that revert or
        // that end the call, and has no frame.
        let Some(return_address_index) = stack.len().checked_sub(parameter_slots + 1) else {
            return;
        };
        let return_address = stack[return_address_index];
        if !u32::try_from(return_address).is_ok_and(|pc| self.context.code.jumpdests.contains(&pc))
        {
            return;
        }

        // A call that returns to where the function that made it returns takes over its frame:
        // the caller does not return.
        if let Some(top) = self.frames.pop_if(|top| {
            top.return_address == return_address && top.return_address_index == return_address_index
        }) {
            self.close(top, i - 1, None);
        }

        // The external function that the dispatcher enters is the call node that the trace
        // shows already. Creation code has no dispatcher, so its calls are internal. In the
        // fallback and receive functions, which no dispatcher enters, a call to a public function
        // is internal unless the call data selects that function. The dispatcher can run inside
        // helper functions, which are not shown. Only the first entry is hidden, so that a public
        // function that calls itself, or another function of the same name, shows its inner calls.
        if !self.context.is_create
            && !self.entered
            && self.frames.iter().all(|frame| frame.function.is_none())
            && function.as_ref().is_some_and(|function| {
                function.is_entry && (self.context.is_dispatched)(&function.declared_name)
            })
        {
            function = None;
            self.entered = true;
        }
        let args = function.as_ref().and_then(|function| {
            let convention = convention?;
            let params = function.params.as_deref()?;
            let words = convention.arguments(stack, return_address_index);
            let memory = entered.memory.as_ref().map(|memory| memory.as_bytes().as_ref());
            decode_parameters(params, &words, memory, Some(self.context.calldata))
        });

        self.frames.push(Frame {
            start: i,
            return_address,
            return_address_index,
            return_slots,
            function,
            args,
        });
    }

    /// Handles the `JUMP` at step `i` that returns from frame `k`.
    fn ret(&mut self, i: usize, k: usize, landing: &CallTraceStep) {
        // The frames above `k` never returned: the call left them by a jump that is not a
        // return. They are dropped.
        self.frames.truncate(k + 1);
        let frame = self.frames.pop().expect("truncated to a frame");
        self.close(frame, i + 1, Some(landing));
    }

    /// Records the call of `frame` that ends at step `end`, with the return values at `landing`,
    /// the step it returned to.
    fn close(&mut self, frame: Frame, end: usize, landing: Option<&CallTraceStep>) {
        let Some(function) = frame.function else { return };
        let return_data = landing.and_then(|landing| {
            let returns = function.returns.as_deref()?;
            let stack = landing.stack.as_deref()?;
            let words = &stack[stack.len().checked_sub(frame.return_slots)?..];
            let memory = landing.memory.as_ref().map(|memory| memory.as_bytes().as_ref());
            decode_parameters(returns, words, memory, None)
        });
        let call =
            DecodedInternalCall { func_name: function.name.clone(), args: frame.args, return_data };
        self.calls.push(InternalCall { start: frame.start, end, call });
    }

    /// The calls that returned. The frames that are still open when the call ends, by a revert,
    /// a halt, or a return from inside the functions, are not shown, as the recorded steps end
    /// before the calls and logs that follow the last jump.
    fn finish(self) -> Vec<InternalCall> {
        self.calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::debug::Param;
    use alloy_dyn_abi::DynSolType;
    use evm2::interpreter::InstrStop;

    /// The start and end step, the name, the arguments and the return values of a call.
    type Summary = (usize, usize, String, Option<Vec<String>>, Option<Vec<String>>);

    const F: u32 = 30;
    const G: u32 = 40;
    const RET: u32 = 50;
    const OTHER_RET: u32 = 60;

    fn step(pc: u32, op: OpCode, stack: &[u64]) -> CallTraceStep {
        CallTraceStep {
            pc,
            op,
            stack: Some(stack.iter().copied().map(U256::from).collect()),
            push_stack: None,
            memory: None,
            gas_remaining: 0,
            gas_refund_counter: 0,
            gas_cost: 0,
            state_gas_reservoir: None,
            storage_change: None,
            status: Some(InstrStop::Stop),
            extra: None,
        }
    }

    /// A `JUMP` to `target` with `stack` below it, and the `JUMPDEST` it lands on.
    fn jump(target: u32, stack: &[u64]) -> [CallTraceStep; 2] {
        let jump = [stack, &[target.into()]].concat();
        [step(1, OpCode::JUMP, &jump), step(target, OpCode::JUMPDEST, stack)]
    }

    fn uint(count: usize) -> Option<Vec<Param>> {
        Some(vec![Param { ty: Some(DynSolType::Uint(256)), storage: None, slots: 1 }; count])
    }

    fn function(name: &str, params: Option<usize>, is_entry: bool) -> FunctionInfo {
        let params = params.and_then(uint);
        FunctionInfo {
            name: name.into(),
            declared_name: name.into(),
            params,
            returns: uint(1),
            is_entry,
        }
    }

    /// Tracks `steps` of code with `f` and `g` that take `parameter_slots` words and return one.
    fn track(
        steps: &[CallTraceStep],
        convention: Option<CallConvention>,
        parameter_slots: usize,
        f: FunctionInfo,
    ) -> Vec<Summary> {
        track_in(steps, convention, parameter_slots, f, (false, &|_| true))
    }

    fn track_in(
        steps: &[CallTraceStep],
        convention: Option<CallConvention>,
        parameter_slots: usize,
        f: FunctionInfo,
        (is_create, is_dispatched): (bool, &dyn Fn(&str) -> bool),
    ) -> Vec<Summary> {
        let entry = || FunctionEntry { name: None, parameter_slots, return_slots: 1 };
        let code = CodeFunctions {
            entries: HashMap::from([(F, vec![entry()]), (G, vec![entry()])]),
            jumpdests: HashSet::from([F, G, RET, OTHER_RET]),
        };
        let g = function("g", Some(parameter_slots), false);
        let resolve = |pc: u32, _: &[FunctionEntry]| {
            let function = if pc == F { &f } else { &g };
            Some((parameter_slots, 1, function.clone()))
        };
        let context = TrackerContext {
            code: &code,
            resolve: &resolve,
            is_create,
            convention,
            calldata: &[],
            is_dispatched,
            from_source_map: &|_| None,
        };
        track_internal_calls(steps, &context)
            .into_iter()
            .map(|c| (c.start, c.end, c.call.func_name, c.call.args, c.call.return_data))
            .collect()
    }

    fn strings(values: &[&str]) -> Option<Vec<String>> {
        Some(values.iter().map(ToString::to_string).collect())
    }

    #[test]
    fn matches_a_return_by_address_and_height() {
        let steps = [
            &jump(F, &[RET.into(), 7])[..],
            // An unmarked jump to the return address at the wrong height is not the return.
            &jump(RET, &[7, 3])[..],
            &jump(RET, &[9])[..],
        ]
        .concat();
        let legacy = Some(CallConvention::SolcLegacy);

        assert_eq!(
            track(&steps, legacy, 1, function("f", Some(1), false)),
            [(0, 5, "f".into(), strings(&["7"]), strings(&["9"]))]
        );
    }

    #[test]
    fn a_jump_without_a_return_label_is_no_frame() {
        // The word below the argument is not a jump destination.
        let steps = [&jump(F, &[5, 7])[..], &jump(RET, &[9])[..]].concat();

        assert_eq!(track(&steps, None, 1, function("f", Some(1), false)), []);
    }

    #[test]
    fn a_call_returning_where_its_caller_returns_takes_over_the_frame() {
        let steps =
            [&jump(F, &[RET.into(), 7])[..], &jump(G, &[RET.into(), 8])[..], &jump(RET, &[9])[..]]
                .concat();
        let calls = track(&steps, None, 1, function("f", Some(1), false));

        // `f` never returns: it ends before the jump that took it over.
        assert_eq!(
            calls,
            [(0, 1, "f".into(), None, None), (2, 5, "g".into(), None, strings(&["9"])),]
        );
    }

    #[test]
    fn hides_the_entry_call_but_not_the_calls_in_it() {
        let steps = [
            &jump(F, &[RET.into(), 7])[..],
            &jump(G, &[OTHER_RET.into(), 8])[..],
            &jump(OTHER_RET, &[9])[..],
            &jump(RET, &[9])[..],
        ]
        .concat();
        let legacy = Some(CallConvention::SolcLegacy);

        assert_eq!(
            track(&steps, legacy, 1, function("f", Some(1), true)),
            [(2, 5, "g".into(), strings(&["8"]), strings(&["9"]))]
        );
    }

    #[test]
    fn a_function_that_the_call_ends_in_is_not_shown() {
        let steps = jump(F, &[RET.into(), 7]);
        let legacy = Some(CallConvention::SolcLegacy);

        assert_eq!(track(&steps, legacy, 1, function("f", Some(1), false)), []);
    }

    #[test]
    fn creation_code_has_no_dispatcher_to_hide_an_entry_call_from() {
        let steps = [&jump(F, &[RET.into(), 7])[..], &jump(RET, &[9])[..]].concat();
        let legacy = Some(CallConvention::SolcLegacy);
        let f = || function("f", Some(1), true);

        assert_eq!(track(&steps, legacy, 1, f()), []);
        assert_eq!(
            track_in(&steps, legacy, 1, f(), (true, &|_| true)),
            [(0, 3, "f".into(), strings(&["7"]), strings(&["9"]))]
        );
    }

    #[test]
    fn only_the_function_that_the_call_data_selects_is_the_dispatcher_entry() {
        let steps = [jump(F, &[RET.into(), 7]), jump(RET, &[9])].concat();
        let legacy = Some(CallConvention::SolcLegacy);
        let f = || function("f", Some(1), true);

        assert_eq!(track_in(&steps, legacy, 1, f(), (false, &|name| name == "f")), []);
        // The call data selects another function, as in a fallback function that calls `f`.
        assert_eq!(
            track_in(&steps, legacy, 1, f(), (false, &|name| name == "g")),
            [(0, 3, "f".into(), strings(&["7"]), strings(&["9"]))]
        );
    }

    #[test]
    fn only_the_first_entry_call_is_hidden() {
        // A public `f` that the dispatcher enters and that calls itself.
        let steps = [
            jump(F, &[RET.into(), 7]),
            jump(F, &[OTHER_RET.into(), 5]),
            jump(OTHER_RET, &[6]),
            jump(RET, &[9]),
        ]
        .concat();
        let legacy = Some(CallConvention::SolcLegacy);

        assert_eq!(
            track(&steps, legacy, 1, function("f", Some(1), true)),
            [(2, 5, "f".into(), strings(&["5"]), strings(&["6"]))]
        );
    }

    #[test]
    fn functions_found_from_the_source_map_report_their_argument_order() {
        // `sub2(10, 3)` enters with `[ret, 3, 10]` if the first argument is on top.
        let steps = [jump(F, &[RET.into(), 3, 10]), jump(RET, &[7])].concat();
        let code = CodeFunctions { entries: HashMap::new(), jumpdests: HashSet::from([F, RET]) };
        let calls = |convention| {
            let from_source_map = |_: usize| {
                Some(SourceMapFunction {
                    function: function("sub2", Some(2), false),
                    parameter_slots: 2,
                    return_slots: 1,
                    convention,
                })
            };
            let context = TrackerContext {
                code: &code,
                resolve: &|_, _| None,
                is_create: false,
                // The convention of the contract does not apply to them.
                convention: Some(CallConvention::SolcLegacy),
                calldata: &[],
                is_dispatched: &|_| true,
                from_source_map: &from_source_map,
            };
            track_internal_calls(&steps, &context)
        };

        // A Yul function takes the first argument on top, like via-IR functions.
        let yul = calls(Some(CallConvention::SolcViaIr));
        assert_eq!(yul[0].call.args, strings(&["10", "3"]));
        assert_eq!(yul[0].call.return_data, strings(&["7"]));
        // A legacy Solidity function takes it at the bottom.
        let solidity = calls(Some(CallConvention::SolcLegacy));
        assert_eq!(solidity[0].call.args, strings(&["3", "10"]));
        assert_eq!(calls(None)[0].call.args, None);
    }

    #[test]
    fn reads_arguments_in_the_order_of_the_pipeline() {
        let legacy = [jump(F, &[RET.into(), 3, 4]), jump(RET, &[0])].concat();
        let via_ir = [jump(F, &[RET.into(), 4, 3]), jump(RET, &[0])].concat();
        let f = || function("f", Some(2), false);

        assert_eq!(
            track(&legacy, Some(CallConvention::SolcLegacy), 2, f())[0].3,
            strings(&["3", "4"])
        );
        assert_eq!(
            track(&via_ir, Some(CallConvention::SolcViaIr), 2, f())[0].3,
            strings(&["3", "4"])
        );
        // Without the pipeline the order is unknown.
        assert_eq!(track(&via_ir, None, 2, f())[0].3, None);
    }

    #[test]
    fn unknown_parameters_leave_arguments_unknown_but_keep_the_call() {
        let steps = [jump(F, &[RET.into(), 3]), jump(RET, &[0])].concat();
        let calls = track(&steps, Some(CallConvention::SolcLegacy), 1, function("f", None, false));

        assert_eq!(calls, [(0, 3, "f".into(), None, strings(&["0"]))]);
    }
}
