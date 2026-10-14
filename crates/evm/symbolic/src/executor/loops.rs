//! Loop summaries for transaction gas bounds.
//!
//! A root-frame jump back to a destination the path already executed in the same internal
//! function activation is a back edge, with the destination as the loop head. Let `S1` be the head
//! state after the first iteration. The summary builds a head state `J` that keeps the values of
//! `S1` that no iteration changes and replaces the others with unknown values, and checks that one
//! iteration from `J` leads back to `J`. Every later head state is then in `J`, so the most gas of
//! one iteration from `J` bounds every later iteration, and the most gas of an exit path from `J`
//! bounds the rest of the call. A stack value that grows by a constant step on every iteration
//! bounds the number of iterations.
//!
//! Every step over-approximates: `SSTORE` is priced without the original values, refunds of the
//! whole transaction are dropped, and storage of each account the loop writes is unknown. If a
//! step fails, the path runs the loop as before for at most [`MAX_UNSUMMARIZED_ITERATIONS`]
//! more iterations, so a summary only changes completeness.

use super::*;
use std::collections::BTreeSet;

/// Most rounds of refining which values change before a summary gives up.
const MAX_SUMMARY_ROUNDS: usize = 6;

/// Most iterations a path runs loops whose summary failed before the run is incomplete.
pub(super) const MAX_UNSUMMARIZED_ITERATIONS: u32 = 32;

/// Most iterations after the first of a loop that runs every iteration instead of its summary,
/// which over-approximates.
pub(super) const MAX_EXACT_ITERATIONS: u64 = 8;

/// Largest step of an induction variable per iteration.
const MAX_INDUCTION_STEP: u64 = 1 << 32;

/// Values that can differ between two arrivals at a loop head.
#[derive(Debug, Default)]
struct Changed {
    /// Stack indices, bottom first.
    stack: BTreeSet<usize>,
    /// Byte offsets of memory words.
    memory: BTreeSet<usize>,
    /// Accounts with written storage.
    storage: BTreeSet<Address>,
}

/// What a candidate induction variable says about the number of iterations.
enum Induction {
    /// At most this many iterations after the first.
    Bounded(u64),
    /// The value grows by a constant step but has no bound: the loop has no iteration bound.
    Unbounded,
    /// The value does not grow by a constant step.
    None,
}

/// Paths from a loop head state.
struct SummaryPaths {
    /// States back at the head after one iteration.
    iterations: Vec<PathState>,
    /// States at the end of the call.
    exits: Vec<PathState>,
}

impl SymbolicExecutor {
    /// Bounds the gas from a loop head to the end of the call, for a path that arrived at the
    /// head `state` after one iteration.
    ///
    /// Returns the reason if the loop cannot be summarized, and the path keeps running it.
    pub(super) fn summarize_loop<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        target: Address,
        state: &PathState,
        completed_paths: &mut usize,
    ) -> Result<Result<SymbolicLoopBound, &'static str>, SymbolicError> {
        let Some(size) = state.memory.concrete_size() else {
            return Ok(Err("symbolic memory size at the loop head"));
        };
        if !self.keeps_max_lengths(executor, target, state)? {
            return Ok(Err("an array can exceed its @custom:max-length at the loop head"));
        }
        let mut changed = Changed::default();
        for _ in 0..MAX_SUMMARY_ROUNDS {
            let epoch = self.loop_epochs;
            self.loop_epochs += 1;
            let start = self.havoc_loop_head(executor, target, state, &changed, epoch)?;
            // An unsupported feature from the unknown head state, like a symbolic jump to an
            // unknown return address, means the head is not a loop head the summary handles.
            let paths = match self.explore_loop(executor, code, &start, completed_paths) {
                Ok(Ok(paths)) => paths,
                Ok(Err(reason)) | Err(SymbolicError::Unsupported(reason)) => {
                    return Ok(Err(reason));
                }
                Err(err) => return Err(err),
            };
            let mut grew = false;
            let writes = start.world.storage.len();
            for iteration in &paths.iterations {
                if iteration.stack.items().len() != start.stack.items().len() {
                    return Ok(Err("the loop changes the stack height"));
                }
                if iteration.memory.concrete_size() != Some(size) {
                    return Ok(Err("the loop expands memory"));
                }
                if iteration.world.transient_storage.len() != start.world.transient_storage.len() {
                    return Ok(Err("the loop writes transient storage"));
                }
                for (index, (before, after)) in
                    start.stack.items().iter().zip(iteration.stack.items()).enumerate()
                {
                    if before != after {
                        grew |= changed.stack.insert(index);
                    }
                }
                for offset in (0..size).step_by(32) {
                    if changed.memory.contains(&offset) {
                        continue;
                    }
                    let before = start.memory.load_word(&mut self.cx, offset)?;
                    let after = iteration.memory.load_word(&mut self.cx, offset)?;
                    if before != after {
                        changed.memory.insert(offset);
                        grew = true;
                    }
                }
                for address in
                    iteration.world.storage.iter().skip(writes).map(|write| write.address)
                {
                    grew |= changed.storage.insert(address);
                }
            }
            if changed.memory.contains(&0x40) {
                return Ok(Err("the loop allocates memory"));
            }
            if !grew {
                return self.bound_loop(executor, target, state, &start, &changed, &paths);
            }
        }
        Ok(Err("the values the loop changes did not settle"))
    }

    /// Returns the loop head state for one round: `state` with every changed value unknown,
    /// storage writes priced without original values, and no refunds.
    fn havoc_loop_head<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: Address,
        state: &PathState,
        changed: &Changed,
        epoch: u64,
    ) -> Result<PathState, SymbolicError> {
        let head = state.pc;
        let mut start = state.clone();
        // Any back edge from the head state is to a nested or an enclosing loop.
        let top = start.activations.len().saturating_sub(1);
        for (depth, activation) in start.activations.iter_mut().enumerate() {
            activation.enter_loop((depth == top).then_some(head));
        }
        start.unsummarized_heads.clear();
        start.back_edge = None;
        for &index in &changed.stack {
            let name = format!("loop{head}_e{epoch}_stack{index}");
            start.stack.items_mut()[index] = SymExpr::var(&mut self.cx, &name);
        }
        for &offset in &changed.memory {
            let name = format!("loop{head}_e{epoch}_memory{offset}");
            let word = SymExpr::var(&mut self.cx, &name);
            start.memory.store_word(&mut self.cx, offset, word);
        }
        for &address in &changed.storage {
            start.world.havoc_storage(address, epoch);
        }
        if changed.storage.contains(&target) {
            for (slot, max) in self.storage_max_lengths.clone() {
                let key = SymExpr::constant(&mut self.cx, slot);
                let length = start.world.sload(&mut self.cx, executor, target, key, Some(slot))?;
                start.constraints.push(SymBoolExpr::cmp_word_const(
                    &mut self.cx,
                    SymCmpOp::Ule,
                    &length,
                    U256::from(max),
                ));
            }
        }
        start.gas.transaction_writes = None;
        start.gas.refunds.clear();
        Ok(start)
    }

    /// Runs every path from the loop head state `start` until it returns to the head or ends
    /// the call. Returns the reason if a path enters another loop.
    fn explore_loop<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        code: &SymCode,
        start: &PathState,
        completed_paths: &mut usize,
    ) -> Result<Result<SummaryPaths, &'static str>, SymbolicError> {
        let head = start.pc;
        let start_depth = start.depth;
        let activations = start.activations.len();
        let path_limit = self.config.path_width() as usize;
        let depth_limit = self.config.execution_depth() as usize;
        let mut worklist = VecDeque::from([start.clone()]);
        let mut deferred_worklist = VecDeque::new();
        let mut paths = SummaryPaths { iterations: Vec::new(), exits: Vec::new() };
        while let Some(next) = self.pop_next_feasible_path(
            &mut worklist,
            &mut deferred_worklist,
            DeferredPathMode::Drain,
        )? {
            let mut state = next.state;
            if *completed_paths >= path_limit {
                return Err(SymbolicError::Unsupported("symbolic path limit exceeded"));
            }
            loop {
                self.check_timeout()?;
                if state.pc == head
                    && state.depth > start_depth
                    && state.activations.len() == activations
                {
                    paths.iterations.push(state);
                    break;
                }
                if state.back_edge.take().is_some() {
                    return Ok(Err("another loop runs before the call ends"));
                }
                if state.depth >= depth_limit {
                    return Err(SymbolicError::Unsupported("symbolic depth limit exceeded"));
                }
                state.depth += 1;
                let outcome = match code.opcode(&mut self.cx, state.pc)? {
                    Some(op) => self.step(
                        executor,
                        code,
                        code.jump_table(),
                        &mut state,
                        &mut worklist,
                        completed_paths,
                        op,
                    )?,
                    None => StepOutcome::Halt,
                };
                match outcome {
                    StepOutcome::Continue => {}
                    StepOutcome::AssumeRejected | StepOutcome::Forked => break,
                    // The bound is over successful transactions.
                    StepOutcome::Revert | StepOutcome::ExceptionalHalt | StepOutcome::Failure => {
                        *completed_paths += 1;
                        break;
                    }
                    StepOutcome::Halt => {
                        *completed_paths += 1;
                        paths.exits.push(state);
                        break;
                    }
                }
            }
        }
        Ok(Ok(paths))
    }

    /// Bounds the loop from its final head state `start`, or returns the reason if no value
    /// bounds the iterations or a path can exceed a declared array length.
    fn bound_loop<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: Address,
        state: &PathState,
        start: &PathState,
        changed: &Changed,
        paths: &SummaryPaths,
    ) -> Result<Result<SymbolicLoopBound, &'static str>, SymbolicError> {
        // A head without a second iteration is not a loop: running it is exact.
        if paths.iterations.is_empty() {
            return Ok(Err("no second iteration"));
        }
        for end in paths.iterations.iter().chain(&paths.exits) {
            if !self.keeps_max_lengths(executor, target, end)? {
                return Ok(Err("the loop can exceed a @custom:max-length"));
            }
        }
        // Induction variable candidates: changed stack values and changed storage words at
        // constant keys, each at the first head arrival, at the unknown head, and after each
        // iteration.
        let mut candidates = Vec::<(SymExpr, SymExpr, Vec<SymExpr>)>::new();
        for &index in &changed.stack {
            let ends =
                paths.iterations.iter().map(|end| end.stack.items()[index].clone()).collect();
            candidates.push((
                state.stack.items()[index].clone(),
                start.stack.items()[index].clone(),
                ends,
            ));
        }
        let writes = start.world.storage.len();
        let slots = paths
            .iterations
            .iter()
            .flat_map(|end| {
                end.world
                    .storage
                    .iter()
                    .skip(writes)
                    .filter(|write| write.address == target)
                    .filter_map(|write| write.key.as_const())
            })
            .collect::<BTreeSet<_>>();
        for slot in slots {
            let load = |cx: &mut SymCx, state: &PathState| {
                let key = SymExpr::constant(cx, slot);
                state.world.clone().sload(cx, executor, target, key, Some(slot))
            };
            let first = load(&mut self.cx, state)?;
            let value = load(&mut self.cx, start)?;
            let ends = paths
                .iterations
                .iter()
                .map(|end| load(&mut self.cx, end))
                .collect::<Result<_, _>>()?;
            candidates.push((first, value, ends));
        }
        let mut iterations = None;
        let mut unbounded = false;
        for (first, value, ends) in candidates {
            match self.iteration_count(state, &first, &value, &paths.iterations, &ends)? {
                Induction::Bounded(count) => {
                    iterations =
                        Some(iterations.map_or(count, |iterations: u64| iterations.min(count)));
                }
                Induction::Unbounded => unbounded = true,
                Induction::None => {}
            }
        }
        let Some(iterations) = iterations else {
            // Running a loop whose counter has no bound does not end either.
            if unbounded {
                return Err(SymbolicError::Unsupported(
                    "loop without an iteration bound; bound its array with @custom:max-length",
                ));
            }
            return Ok(Err("no counter grows by a constant step"));
        };
        let mark = (start.gas.spent, start.gas.terms.len());
        let mut iteration = 0;
        for end in &paths.iterations {
            let gas = end.gas.since(&mut self.cx, mark)?;
            iteration = iteration.max(self.max_path_gas(end, &gas)?);
        }
        let mut exit = 0;
        for end in &paths.exits {
            let gas = end.gas.since(&mut self.cx, mark)?;
            exit = exit.max(self.max_path_gas(end, &gas)?);
        }
        Ok(Ok(SymbolicLoopBound { pc: state.pc, iteration, iterations, exit }))
    }

    /// Returns the most iterations after the first, if `value` at the unknown head grows by a
    /// constant step to `ends` on every iteration path and is bounded on every iteration path.
    ///
    /// Summarized iteration `k` starts with the value `first + (k - 1) * step`, where `first` is
    /// the value after the first iteration, and needs it at most `max`. So
    /// `k <= (max - first) / step + 1` for a known `first`, and `k <= max / step + 1` otherwise,
    /// or `k <= max / step` if `first >= step`.
    fn iteration_count(
        &mut self,
        state: &PathState,
        first: &SymExpr,
        value: &SymExpr,
        iterations: &[PathState],
        ends: &[SymExpr],
    ) -> Result<Induction, SymbolicError> {
        let mut step = u64::MAX;
        let mut max = 0;
        for (end, next) in iterations.iter().zip(ends) {
            let delta = SymExpr::binop(&mut self.cx, SymBinOp::Sub, next.clone(), value.clone());
            let delta = match delta.as_const() {
                Some(delta) => delta,
                None => {
                    let replayable_storage = end.world.replay_storage_symbols();
                    let model = self.solver.model_with_replayable_storage(
                        &mut self.cx,
                        &end.constraints,
                        &replayable_storage,
                    )?;
                    let Some(delta_value) = delta.eval_model_if_complete(&model)? else {
                        return Ok(Induction::None);
                    };
                    let mut constraints = end.constraints.clone();
                    constraints.push(
                        SymBoolExpr::eq_word_const(&mut self.cx, &delta, delta_value)
                            .not(&mut self.cx),
                    );
                    if self.is_sat_with_state(end, &constraints)? {
                        return Ok(Induction::None);
                    }
                    delta_value
                }
            };
            let Some(delta) =
                u64::try_from(delta).ok().filter(|delta| (1..=MAX_INDUCTION_STEP).contains(delta))
            else {
                return Ok(Induction::None);
            };
            step = step.min(delta);
            match self.max_path_gas(end, value) {
                Ok(value) => max = max.max(value),
                Err(SymbolicError::Unsupported(_)) => return Ok(Induction::Unbounded),
                Err(err) => return Err(err),
            }
        }
        if let Some(first) = first.as_const() {
            let max = U256::from(max);
            return Ok(Induction::Bounded(if first > max {
                0
            } else {
                u64::try_from((max - first) / U256::from(step)).unwrap_or(u64::MAX) + 1
            }));
        }
        let mut constraints = state.constraints.clone();
        constraints.push(SymBoolExpr::cmp_word_const(
            &mut self.cx,
            SymCmpOp::Ult,
            first,
            U256::from(step),
        ));
        let below_step = self.is_sat_with_state(state, &constraints)?;
        Ok(Induction::Bounded(max / step + u64::from(below_step)))
    }

    /// Returns whether every bounded array length of `target` is at most its maximum on every
    /// input of the path.
    fn keeps_max_lengths<FEN: FoundryEvmNetwork>(
        &mut self,
        executor: &Executor<FEN>,
        target: Address,
        state: &PathState,
    ) -> Result<bool, SymbolicError> {
        for (slot, max) in self.storage_max_lengths.clone() {
            let key = SymExpr::constant(&mut self.cx, slot);
            let mut world = state.world.clone();
            let length = world.sload(&mut self.cx, executor, target, key, Some(slot))?;
            let mut constraints = state.constraints.clone();
            constraints.push(SymBoolExpr::cmp_word_const(
                &mut self.cx,
                SymCmpOp::Ugt,
                &length,
                U256::from(max),
            ));
            if self.is_sat_with_state(state, &constraints)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}
