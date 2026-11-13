use super::{
    CoverageItemKind, ExecutionAnchor, ExecutionAnchorKind, ItemAnchor, ItemAnchors, SourceLocation,
};
use crate::analysis::{BranchKind, EmptySpecialFunctionKind, IfArms, SourceAnalysis, TernaryArms};
use alloy_primitives::{U256, map::rustc_hash::FxHashSet};
use evm2::interpreter::op as opcode;
use eyre::ensure;
use foundry_compilers::artifacts::sourcemap::{Jump, SourceElement, SourceMap};
use foundry_evm_core::{bytecode::InstIter, ic::IcPcMap};
use std::ops::Range;

impl ItemAnchor {
    fn from_instruction(
        instruction: Option<usize>,
        ic_pc_map: &IcPcMap,
        item_id: u32,
        loc: &SourceLocation,
    ) -> eyre::Result<Self> {
        let instruction = instruction.ok_or_else(|| {
            eyre::eyre!("Could not find anchor: No matching instruction in range {loc}")
        })?;
        Ok(Self {
            instruction: ic_pc_map.get(instruction as u32).ok_or_else(|| {
                eyre::eyre!("We found an anchor, but we can't translate it to a program counter")
            })?,
            item_id,
        })
    }

    fn from_branch(item_id: u32, next_pc: usize, immediate: &[u8]) -> eyre::Result<(Self, Self)> {
        let pc_jump = jump_destination(immediate)?;
        Ok((
            Self { item_id, instruction: (next_pc + 1).try_into()? },
            Self { item_id, instruction: pc_jump },
        ))
    }
}

/// An anchor with the conditional jump whose taken edge it counts and the jumps that enter the
/// fallthrough from elsewhere, as in [`ItemAnchors::jumps`] and [`ItemAnchors::entries`].
type EdgeAnchor = (ItemAnchor, Option<u32>, Vec<(u32, bool)>);

/// Decodes a push immediate as a program counter.
fn jump_destination(immediate: &[u8]) -> eyre::Result<u32> {
    // We do not support program counters bigger than u32.
    ensure!(immediate.len() <= 4, "jump destination overflow");
    let mut pc_bytes = [0u8; 4];
    pc_bytes[4 - immediate.len()..].copy_from_slice(immediate);
    Ok(u32::from_be_bytes(pc_bytes))
}

/// Attempts to find anchors for the given items using the given source map and bytecode.
pub fn find_anchors(
    bytecode: &[u8],
    source_map: &SourceMap,
    ic_pc_map: &IcPcMap,
    analysis: &SourceAnalysis,
) -> ItemAnchors {
    let mut anchors = ItemAnchors::default();
    let index = AnchorIndex::new(bytecode, source_map, ic_pc_map);
    // Path 0 is the fallthrough and path 1 the jump target, unless `swapped`. The jump target
    // counts the jump's executions minus its fallthrough's.
    let select_branch = |(fallthrough, taken): (ItemAnchor, ItemAnchor), path_id, swapped| {
        let jump = Some(fallthrough.instruction - 1);
        match (path_id, swapped) {
            (0, false) | (1, true) => (fallthrough, None, Vec::new()),
            (1, false) | (0, true) => (taken, jump, Vec::new()),
            _ => panic!("too many path IDs for branch"),
        }
    };
    let mut seen_sources = FxHashSet::default();
    source_map
        .iter()
        .filter_map(|element| element.index())
        .filter(|&source| seen_sources.insert(source))
        .flat_map(|source| analysis.items_for_source_enumerated(source))
        .filter_map(|(item_id, item)| {
            if analysis.empty_special_function_kind(item_id).is_some() {
                return None;
            }
            let anchor_loc = item.anchor_loc.as_ref().unwrap_or(&item.loc);
            let recover = || {
                let hints = analysis.anchor_hints(item_id)?;
                if let Some(guard) = &hints.revert_guard
                    && let Some((anchor, jump)) = index.revert_guard(item_id, guard)
                {
                    return Some((anchor, jump, vec![]));
                }
                hints.alias_function.as_ref().and_then(|function| {
                    index.alias_return(item_id, function).map(|anchor| (anchor, None, vec![]))
                }).or_else(|| {
                    hints.unique_callee.as_ref().and_then(|callee| {
                        index.unique_callee_entry(item_id, callee).map(|anchor| (anchor, None, vec![]))
                    })
                })
            };
            if let CoverageItemKind::Branch { branch_id, path_id, .. } = item.kind
                && let Some(BranchKind::If(arms)) =
                    analysis.branch_kind(item.loc.source_id as u32, branch_id)
            {
                let arm = if path_id == 0 { Some(&arms.then_arm) } else { arms.else_arm.as_ref() };
                // The first instruction of an arm with code anchors it. Without code, only the
                // jump's edge can count it, and when that is ambiguous the arm stays unanchored
                // instead of taking an instruction that runs with the jump, such as its target.
                if !arm.is_some_and(|arm| index.has_own_code(arm)) {
                    return index.if_edge(item_id, path_id, &item.loc, arms).or_else(recover);
                }
            }
            match item.kind {
                CoverageItemKind::Branch { path_id: 1, is_first_opcode: true, .. }
                    if item.anchor_loc.is_some() =>
                {
                    index.simple(item_id, anchor_loc).map(|anchor| (anchor, None, vec![])).or_else(
                        |_| {
                            index
                                .branch(item_id, &item.loc, false, None)
                                .map(|anchors| select_branch(anchors, 1, false))
                        },
                    )
                }
                CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: false } => {
                    let kind = analysis.branch_kind(item.loc.source_id as u32, branch_id);
                    let exact = matches!(kind, Some(BranchKind::Ternary(_)));
                    // A Yul `if` owns the jump outside its body, not those of nested branches.
                    let body = match kind {
                        Some(BranchKind::YulIf(body)) => Some(body),
                        _ => None,
                    };
                    index.branch(item_id, anchor_loc, exact, body).map(|anchors| {
                        let entries = if body.is_some() {
                            index.fallthrough_entries(&anchors).unwrap_or_default()
                        } else {
                            vec![]
                        };
                        // The failing side of a `require`, the false arm of a ternary and the body
                        // of a Yul `if` are path 0, wherever the compiler placed them.
                        let swapped = match kind {
                            Some(BranchKind::Require) => index.taken_fails(&anchors, anchor_loc),
                            Some(BranchKind::Ternary(arms)) => {
                                index.true_arm_falls_through(&anchors, arms)
                            }
                            Some(BranchKind::YulIf(body)) => index
                                .taken_starts_in(&anchors, body)
                                .unwrap_or_else(|| index.jump_is_body(&anchors, body)),
                            None | Some(BranchKind::If(_)) => false,
                        };
                        let (anchor, jump, _) = select_branch(anchors, path_id, swapped);
                        (anchor, jump, entries)
                    })
                }
                CoverageItemKind::Function { .. } => {
                    index.function(item_id, anchor_loc).map(|anchor| (anchor, None, vec![]))
                }
                _ => index.simple(item_id, anchor_loc).map(|anchor| (anchor, None, vec![])),
            }
            .or_else(|error| recover().ok_or(error))
            .inspect_err(|err| warn!(%item, %err, "could not find anchor"))
            .ok()
        })
        .for_each(|(anchor, jump, entries)| {
            if let Some(jump) = jump {
                anchors.jumps.insert(anchors.anchors.len(), jump);
            }
            if !entries.is_empty() {
                anchors.entries.insert(anchors.anchors.len(), entries);
            }
            anchors.anchors.push(anchor);
        });
    anchors
}

/// Finds execution-based anchors for empty constructors, receive functions, and fallbacks in a
/// contract.
pub fn find_execution_anchors(
    source_id: u32,
    contract_name: &str,
    analysis: &SourceAnalysis,
) -> Vec<ExecutionAnchor> {
    analysis
        .empty_special_function_ids(source_id, contract_name)
        .filter_map(|item_id| {
            analysis.empty_special_function_kind(item_id).map(|kind| ExecutionAnchor {
                item_id,
                kind: match kind {
                    EmptySpecialFunctionKind::Constructor => ExecutionAnchorKind::Constructor,
                    EmptySpecialFunctionKind::Receive => ExecutionAnchorKind::Receive,
                    EmptySpecialFunctionKind::Fallback => ExecutionAnchorKind::Fallback,
                },
            })
        })
        .collect()
}

/// The most static jumps `AnchorIndex::trace` follows.
const MAX_TRACE_HOPS: usize = 8;

/// Source map and branch lookups precomputed once per bytecode.
struct AnchorIndex<'a> {
    bytecode: &'a [u8],
    source_map: &'a SourceMap,
    /// The program counter of every instruction, by instruction counter.
    pcs: Vec<u32>,
    ic_pc_map: &'a IcPcMap,
    /// `(source id, offset, instruction)`, sorted for source range lookups.
    entries: Vec<(i32, u32, usize)>,
    /// `(ic, next pc, push immediate)` for every push that feeds a `JUMPI`.
    branches: Vec<(usize, usize, &'a [u8])>,
    /// Straight-line instruction ranges ending in REVERT or INVALID, ordered by start.
    reverting_runs: Vec<Range<usize>>,
    /// `(JUMPDEST, start of its straight-line run)` for every `JUMPDEST` that the previous
    /// instruction falls through to, ordered. A run starts after a `JUMP`, `STOP`, `RETURN`,
    /// `REVERT`, `INVALID`, `SELFDESTRUCT`, or a `JUMPI` that can skip to something other than a
    /// failure stub or a block that jumps to one. A `JUMP` that no source element maps does not
    /// end a run.
    fallen_into: Vec<(usize, usize)>,
    /// `(target pc, push instruction)` for every push that feeds a `JUMP` or `JUMPI`, sorted.
    jumps: Vec<(u32, usize)>,
    /// `(target pc, push instruction)` for every push of a `JUMPDEST` address, sorted. Unlike
    /// `jumps`, this includes return addresses that code pushes before jumping to a helper.
    address_pushes: Vec<(u32, usize)>,
    /// Whether execution can fall through from the previous instruction into each instruction.
    falls_through: Vec<bool>,
}

impl<'a> AnchorIndex<'a> {
    fn new(bytecode: &'a [u8], source_map: &'a SourceMap, ic_pc_map: &'a IcPcMap) -> Self {
        let mut entries = source_map
            .iter()
            .enumerate()
            .map(|(instruction, element)| (element.index_i32(), element.offset(), instruction))
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut pcs = Vec::new();
        let mut branches = Vec::new();
        let mut reverting_runs = Vec::new();
        let mut jumps = Vec::new();
        // `(value, push instruction, feeds a jump)` for every push of at most four bytes.
        let mut pushes = Vec::new();
        let mut falls_through = Vec::new();
        let mut opcodes = Vec::new();
        // `(pc, instruction)` of every `JUMPDEST`.
        let mut jumpdests = Vec::new();
        // Instructions that end a straight-line run, and `(JUMPI, static target pc)` pairs.
        let mut run_ends = Vec::new();
        let mut conditional_jumps = Vec::new();
        // `(run, static target pc)` of every run ending in a `JUMP`.
        let mut jump_runs = Vec::new();
        let mut run_start = 0;
        let mut prev = opcode::STOP;
        let mut prev_target = None;
        for (ic, (pc, inst)) in InstIter::new(bytecode).with_pc().enumerate() {
            opcodes.push(inst.opcode.get());
            falls_through.push(!matches!(
                prev,
                opcode::JUMP
                    | opcode::STOP
                    | opcode::RETURN
                    | opcode::REVERT
                    | opcode::INVALID
                    | opcode::SELFDESTRUCT
            ));
            pcs.push(pc as u32);
            match inst.opcode.get() {
                opcode::JUMPDEST => jumpdests.push((pc, ic)),
                opcode::JUMPI => conditional_jumps.push((ic, prev_target)),
                opcode::JUMP => {
                    jump_runs.push((run_start..ic + 1, prev_target));
                    // A compiler-generated helper, such as an ABI decoder, returns to the code
                    // that called it, so its `JUMP` does not end the caller's run.
                    if source_map.get(ic).is_none_or(|e| e.index().is_some()) {
                        run_ends.push(ic);
                    }
                }
                opcode::STOP
                | opcode::RETURN
                | opcode::REVERT
                | opcode::INVALID
                | opcode::SELFDESTRUCT => run_ends.push(ic),
                _ => {}
            }
            prev_target = (opcode::PUSH1..=opcode::PUSH4)
                .contains(&inst.opcode.get())
                .then(|| inst.immediate.iter().fold(0, |acc, &byte| acc << 8 | byte as usize));
            match inst.opcode.get() {
                opcode::REVERT | opcode::INVALID => {
                    reverting_runs.push(run_start..ic + 1);
                    run_start = ic + 1;
                }
                // A JUMPDEST begins a basic block, so a failure stub led by one is part of its run.
                // Adjacent JUMPDESTs share the run started by the first.
                opcode::JUMPDEST if prev != opcode::JUMPDEST => run_start = ic,
                opcode::JUMP
                | opcode::JUMPI
                | opcode::STOP
                | opcode::RETURN
                | opcode::SELFDESTRUCT => run_start = ic + 1,
                _ => {}
            }
            prev = inst.opcode.get();
            if (opcode::PUSH1..=opcode::PUSH32).contains(&inst.opcode.get()) {
                // Stop where the source map ends.
                if source_map.get(ic).is_none() {
                    break;
                }
                let next_pc = pc + inst.immediate.len() + 1;
                let jump = bytecode.get(next_pc).copied();
                if jump == Some(opcode::JUMPI) {
                    branches.push((ic, next_pc, inst.immediate));
                }
                if inst.immediate.len() <= 4 {
                    let target = inst.immediate.iter().fold(0, |acc, &byte| acc << 8 | byte as u32);
                    let feeds_jump = matches!(jump, Some(opcode::JUMP | opcode::JUMPI));
                    pushes.push((target, ic, feeds_jump));
                    if feeds_jump {
                        jumps.push((target, ic));
                    }
                }
            }
        }
        jumps.sort_unstable();
        let jumpdest_at = |pc: Option<usize>| {
            pc.and_then(|pc| jumpdests.binary_search_by_key(&pc, |&(pc, _)| pc).ok())
                .map(|i| jumpdests[i].1)
        };
        // A push of a `JUMPDEST` address is a jump or a return address when it feeds a jump, or
        // when the code after it reaches a `JUMP` or `JUMPI` before it halts, as a call does. Other
        // pushes are constants.
        let mut reaches_jump = vec![false; opcodes.len()];
        let mut next = false;
        for (ic, &op) in opcodes.iter().enumerate().rev() {
            next = match op {
                opcode::JUMP | opcode::JUMPI => true,
                opcode::STOP
                | opcode::RETURN
                | opcode::REVERT
                | opcode::INVALID
                | opcode::SELFDESTRUCT => false,
                _ => next,
            };
            reaches_jump[ic] = next;
        }
        let mut address_pushes = pushes
            .into_iter()
            .filter(|&(target, ic, feeds_jump)| {
                jumpdest_at(Some(target as usize)).is_some() && (feeds_jump || reaches_jump[ic])
            })
            .map(|(target, ic, _)| (target, ic))
            .collect::<Vec<_>>();
        address_pushes.sort_unstable();
        // A run that jumps to a failure stub, such as a panic helper call, is a failure stub too.
        let failing_jump_runs = jump_runs
            .into_iter()
            .filter(|&(_, target)| {
                jumpdest_at(target).is_some_and(|ic| in_reverting_run(&reverting_runs, ic))
            })
            .map(|(run, _)| run)
            .collect::<Vec<_>>();
        // A `JUMPI` ends a run unless it can only skip to a failure stub.
        run_ends.extend(conditional_jumps.into_iter().filter_map(|(ic, target)| {
            let fails = jumpdest_at(target).is_some_and(|ic| {
                in_reverting_run(&reverting_runs, ic) || in_reverting_run(&failing_jump_runs, ic)
            });
            (!fails).then_some(ic)
        }));
        run_ends.sort_unstable();
        let fallen_into = jumpdests
            .iter()
            .filter_map(|&(_, ic)| {
                let run_start = run_ends.partition_point(|&end| end < ic).checked_sub(1);
                let run_start = run_start.map_or(0, |i| run_ends[i] + 1);
                (run_start < ic).then_some((ic, run_start))
            })
            .collect();
        Self {
            bytecode,
            source_map,
            pcs,
            ic_pc_map,
            entries,
            branches,
            reverting_runs,
            fallen_into,
            jumps,
            address_pushes,
            falls_through,
        }
    }

    /// Returns the instructions whose source elements start in the byte range of `loc`.
    fn instructions_starting_in(
        &self,
        loc: &SourceLocation,
    ) -> impl Iterator<Item = usize> + Clone {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let end = self.entries.partition_point(|&(id, offset, _)| {
            (id, offset) <= (source, loc.bytes.start + loc.len())
        });
        self.entries[start..end].iter().map(|&(_, _, instruction)| instruction)
    }

    /// Finds the entry opcode of the source range, preferring code outside a failure stub.
    ///
    /// The entry is the lowest instruction in the range that is not provably reached only from
    /// inside the range. A compiler can lay out blocks that belong to the range before its entry,
    /// such as the cleanup of a failed condition, and only code inside the range reaches those.
    /// Dynamic jumps are not tracked, so an instruction with no known way in, such as the entry of
    /// an internal function, counts as an entry.
    fn simple(&self, item_id: u32, loc: &SourceLocation) -> eyre::Result<ItemAnchor> {
        let candidates = self
            .instructions_starting_in(loc)
            .filter(|&instruction| is_in_source_range(&self.source_map[instruction], loc));
        // Cold validation stubs can precede the statement's main code in the bytecode.
        // A revert statement itself may have only failure-path candidates, so retain the fallback.
        let mut live = candidates
            .clone()
            .filter(|&instruction| !in_reverting_run(&self.reverting_runs, instruction))
            .collect::<Vec<_>>();
        live.sort_unstable();
        let instruction = live
            .iter()
            .copied()
            .find(|&instruction| !self.is_entered_only_from_inside(instruction, loc))
            .or_else(|| live.first().copied())
            .or_else(|| candidates.min());
        ItemAnchor::from_instruction(instruction, self.ic_pc_map, item_id, loc)
    }

    /// Returns whether the taken side of a `require` branch is its failing side.
    ///
    /// A side that starts inside the `require` span is the failing side, because the failure
    /// code carries the span while the passing side continues with the next statement. If
    /// neither side starts inside it, the side that reverts without another conditional branch
    /// is. If both do, the source map cannot tell them apart, and the revert check cannot follow
    /// the failure code's dynamic returns, so the positional default stays.
    fn taken_fails(&self, anchors: &(ItemAnchor, ItemAnchor), loc: &SourceLocation) -> bool {
        self.taken_starts_in(anchors, loc).unwrap_or_else(|| {
            let (fallthrough, taken) = anchors;
            self.fails(taken.instruction) && !self.fails(fallthrough.instruction)
        })
    }

    /// Returns whether the taken side starts inside `loc` and the fallthrough does not, or `None`
    /// when neither does. A fallthrough that starts inside `loc` wins, so the positional default
    /// stays when both do.
    fn taken_starts_in(
        &self,
        (fallthrough, taken): &(ItemAnchor, ItemAnchor),
        loc: &SourceLocation,
    ) -> Option<bool> {
        match (self.starts_in(fallthrough.instruction, loc), self.starts_in(taken.instruction, loc))
        {
            (true, _) => Some(false),
            (false, true) => Some(true),
            (false, false) => None,
        }
    }

    /// Returns whether the jump's own target push is mapped to `loc`, as when a body that is
    /// only a `break`, `continue`, or `leave` becomes the conditional jump itself.
    fn jump_is_body(
        &self,
        (fallthrough, _): &(ItemAnchor, ItemAnchor),
        loc: &SourceLocation,
    ) -> bool {
        let jump = self.pcs.binary_search(&(fallthrough.instruction - 1)).ok();
        jump.is_some_and(|jump| self.starts_in_range(jump - 1, loc))
    }

    /// Returns whether the first instruction at `pc` after any `JUMPDEST`s is mapped inside `loc`.
    fn starts_in(&self, pc: u32, loc: &SourceLocation) -> bool {
        let Ok(mut ic) = self.pcs.binary_search(&pc) else { return false };
        while self.pcs.get(ic).is_some_and(|&p| self.bytecode[p as usize] == opcode::JUMPDEST) {
            ic += 1;
        }
        self.maps_inside(ic, loc)
    }

    /// Returns whether the instruction `ic` is mapped inside `loc`.
    fn maps_inside(&self, ic: usize, loc: &SourceLocation) -> bool {
        self.source_map.get(ic).is_some_and(|element| is_within_source_span(element, loc))
    }

    /// Returns whether the code at `pc` reverts without another conditional branch or other halt.
    ///
    /// Follows static `PUSH target; JUMP` hops, which compilers use to share revert blocks.
    fn fails(&self, pc: u32) -> bool {
        let last = self.trace(pc).last().map(|&ic| self.bytecode[self.pcs[ic] as usize]);
        matches!(last, Some(opcode::REVERT | opcode::INVALID))
    }

    /// Returns whether the true arm of a ternary is the fallthrough of its jump.
    ///
    /// A successor with code mapped inside one arm's span is that arm, because the arms carry their
    /// own spans while the join point carries the enclosing expression's. An arm without code, such
    /// as a literal already on the stack, has no successor of its own, so the other arm decides.
    /// If neither arm has code, or the successors give conflicting answers, the positional default
    /// stays: the true arm is the jump target.
    fn true_arm_falls_through(
        &self,
        (fallthrough, taken): &(ItemAnchor, ItemAnchor),
        arms: &TernaryArms,
    ) -> bool {
        let holds = |pc, arm| self.block_maps_inside(pc, arm);
        let true_fallthrough = holds(fallthrough.instruction, &arms.true_arm);
        let true_taken = holds(taken.instruction, &arms.true_arm);
        let false_fallthrough = holds(fallthrough.instruction, &arms.false_arm);
        let false_taken = holds(taken.instruction, &arms.false_arm);
        let falls_through =
            (true_fallthrough && !true_taken) || (false_taken && !false_fallthrough);
        let is_taken = (true_taken && !true_fallthrough) || (false_fallthrough && !false_taken);
        falls_through && !is_taken
    }

    /// Returns whether an instruction of the block at `pc` is mapped inside `loc`. The block runs
    /// to the first jump or halt, so it covers the entry code of an arm, which can start with
    /// instructions mapped to the enclosing expression. An arm that falls into the join point
    /// also scans the code after it, which is mapped outside both arms.
    fn block_maps_inside(&self, pc: u32, loc: &SourceLocation) -> bool {
        let Ok(start) = self.pcs.binary_search(&pc) else { return false };
        for (ic, &next) in self.pcs.iter().enumerate().skip(start) {
            if self.source_map.get(ic).is_some_and(|element| is_within_source_span(element, loc)) {
                return true;
            }
            if matches!(
                self.bytecode[next as usize],
                opcode::JUMP
                    | opcode::JUMPI
                    | opcode::STOP
                    | opcode::RETURN
                    | opcode::REVERT
                    | opcode::INVALID
                    | opcode::SELFDESTRUCT
            ) {
                break;
            }
        }
        false
    }

    /// Finds the entry of a function.
    ///
    /// Compilers map the entry `JUMPDEST` to exactly the function's span, so it is preferred over
    /// the lowest instruction in the span, which can belong to a body block laid out before the
    /// entry. The entry is the exact-span instruction that code outside the function jumps to,
    /// which excludes the function's exit label and inlined code. A public function can have two
    /// such instructions, its ABI wrapper and its body. The wrapper falls through into the body,
    /// which internal calls jump to, so the body runs on both paths and is preferred. Falls back
    /// to [`Self::simple`] when there is none, as for inlined constructors and fallbacks, and for
    /// functions that only their own modifier wrapper jumps to, such as with solc via-IR and the
    /// optimizer.
    fn function(&self, item_id: u32, loc: &SourceLocation) -> eyre::Result<ItemAnchor> {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let candidates = self.entries[start..]
            .iter()
            .take_while(|&&(id, offset, _)| (id, offset) == (source, loc.bytes.start))
            .map(|&(_, _, instruction)| instruction)
            .filter(|&instruction| {
                self.source_map[instruction].length() == loc.len()
                    && self.is_jumped_to_from_outside(instruction, loc)
            })
            .collect::<Vec<_>>();
        // The lowest candidate can fall through into a later one.
        let entry = candidates.iter().copied().min().map(|lowest| {
            candidates
                .iter()
                .copied()
                .filter(|&instruction| self.falls_through_into(lowest, instruction))
                .min()
                .unwrap_or(lowest)
        });
        match entry {
            Some(_) => ItemAnchor::from_instruction(entry, self.ic_pc_map, item_id, loc),
            None => self.simple(item_id, loc),
        }
    }

    /// Returns whether execution can fall through from `from` into the `JUMPDEST` `to`.
    fn falls_through_into(&self, from: usize, to: usize) -> bool {
        self.fallen_into
            .binary_search_by_key(&to, |&(jumpdest, _)| jumpdest)
            .is_ok_and(|i| (self.fallen_into[i].1..to).contains(&from))
    }

    /// Returns whether every known way into `instruction` comes from an instruction mapped inside
    /// `loc`, and there is at least one. The known ways in are falling through and a push of its
    /// address, which is a static jump or a return address. Unmapped instructions, such as the
    /// `JUMPDEST` that a compiler puts before a loop body, belong to the block they lead, so the
    /// ways into that block count.
    fn is_entered_only_from_inside(&self, instruction: usize, loc: &SourceLocation) -> bool {
        let inside = |element: &SourceElement| is_in_source_range(element, loc);
        let (mut any, mut all) = (false, true);
        let mut ic = instruction;
        loop {
            if let Some(pc) = self.ic_pc_map.get(ic as u32) {
                let start = self.address_pushes.partition_point(|&(target, _)| target < pc);
                for &(_, push) in
                    self.address_pushes[start..].iter().take_while(|&&(target, _)| target == pc)
                {
                    any = true;
                    all &= self.source_map.get(push).is_some_and(inside);
                }
            }
            let Some(prev) = ic.checked_sub(1) else {
                // Execution starts at the first instruction.
                return false;
            };
            if !self.falls_through.get(ic).copied().unwrap_or(false) {
                break;
            }
            match self.source_map.get(prev) {
                Some(element) if element.index().is_none() => ic = prev,
                element => {
                    any = true;
                    all &= element.is_some_and(inside);
                    break;
                }
            }
        }
        any && all
    }

    /// Returns whether a static jump mapped outside `loc` targets `instruction`.
    fn is_jumped_to_from_outside(&self, instruction: usize, loc: &SourceLocation) -> bool {
        let Some(pc) = self.ic_pc_map.get(instruction as u32) else { return false };
        let start = self.jumps.partition_point(|&(target, _)| target < pc);
        self.jumps[start..]
            .iter()
            .take_while(|&&(target, _)| target == pc)
            .any(|&(_, push)| !is_within_source_span(&self.source_map[push], loc))
    }

    /// Returns the jumps that enter the fallthrough of a branch from elsewhere, as `(pc,
    /// conditional)`, such as the back edge of a loop whose head the branch falls into, or a
    /// `continue` that jumps to the code after the `if`. Their executions are not the
    /// fallthrough's, but its hits count them. Fails when the entries cannot be told apart: the
    /// branch's own jump targets the fallthrough, or a conditional jump's own fallthrough is a
    /// `JUMPDEST` that others can enter. The branch then keeps its anchors without a discount.
    fn fallthrough_entries(
        &self,
        (fallthrough, _): &(ItemAnchor, ItemAnchor),
    ) -> eyre::Result<Vec<(u32, bool)>> {
        let pc = fallthrough.instruction;
        if self.bytecode.get(pc as usize) != Some(&opcode::JUMPDEST) {
            return Ok(Vec::new());
        }
        let start = self.jumps.partition_point(|&(target, _)| target < pc);
        self.jumps[start..]
            .iter()
            .take_while(|&&(target, _)| target == pc)
            .map(|&(_, push)| {
                let jump = self.pcs.get(push + 1).copied().unwrap_or_default();
                let conditional = self.bytecode[jump as usize] == opcode::JUMPI;
                ensure!(
                    jump != pc - 1
                        && (!conditional
                            || self.bytecode.get(jump as usize + 1) != Some(&opcode::JUMPDEST)),
                    "fallthrough entries cannot be told apart"
                );
                Ok((jump, conditional))
            })
            .collect()
    }

    /// Finds the fallthrough and taken anchors of a branch.
    ///
    /// `exact` matches only the jump mapped to exactly `loc`, so ternaries skip nested decisions.
    /// Otherwise the last matching jump wins, preferring one mapped outside `nested`, the code
    /// of the branch's body, which holds the jumps of nested branches and loops.
    fn branch(
        &self,
        item_id: u32,
        loc: &SourceLocation,
        exact: bool,
        nested: Option<&SourceLocation>,
    ) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
        let mut own = None;
        let mut anchors = None;
        for &(ic, next_pc, immediate) in &self.branches {
            let matches = if exact {
                self.source_map.get(ic + 1).is_some_and(|jump| {
                    jump.index() == Some(loc.source_id as u32)
                        && jump.offset() == loc.bytes.start
                        && jump.length() == loc.len()
                })
            } else {
                is_in_source_range(&self.source_map[ic], loc)
            };
            if matches {
                let found = ItemAnchor::from_branch(item_id, next_pc, immediate)?;
                // Generated branch code can contain later jumps mapped to the ternary span.
                if exact {
                    return Ok(found);
                }
                if nested.is_none_or(|nested| !self.maps_inside(ic + 1, nested)) {
                    own = Some(found);
                } else {
                    anchors = Some(found);
                }
            }
        }
        own.or(anchors).ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
    }

    /// Finds the anchor that counts the entries into an arm of an `if` that has no code of its
    /// own, from the edge of the conditional jump that enters it, as `gcov` counts arcs.
    ///
    /// The `if`'s jump is the only one mapped inside the statement but outside its arms, which
    /// hold the jumps of nested branches. Fails when the side that enters the arm is ambiguous
    /// (see [`Self::then_is_taken`]), or when the fallthrough can be entered other than by falling
    /// through, so that its hits do not split the jump's.
    fn if_edge(
        &self,
        item_id: u32,
        path_id: u32,
        loc: &SourceLocation,
        arms: &IfArms,
    ) -> Option<EdgeAnchor> {
        // The statement's span ends before the `;` that a jump's source element can include.
        let mut jumps = self
            .branches
            .iter()
            .filter(|&&(ic, ..)| {
                self.source_map.get(ic + 1).is_some_and(|jump| {
                    is_in_source_range(jump, loc)
                        && !is_in_source_range(jump, &arms.then_arm)
                        && arms.else_arm.as_ref().is_none_or(|arm| !is_in_source_range(jump, arm))
                })
            })
            .collect::<Vec<_>>();
        // A jump to a failure, such as an overflow check in the condition, cannot enter an arm,
        // unless it is the only jump, as when the arm is a shared `revert`.
        if jumps.len() > 1 {
            jumps.retain(|&&(_, _, immediate)| {
                jump_destination(immediate).is_ok_and(|target| !self.fails(target))
            });
        }
        let &[&(_, jump, immediate)] = jumps.as_slice() else { return None };
        let anchors = ItemAnchor::from_branch(item_id, jump, immediate).ok()?;
        let (fallthrough, taken) = &anchors;
        let pc = fallthrough.instruction;
        let pushes = |pushes: &[(u32, usize)]| {
            pushes.partition_point(|&(target, _)| target <= pc)
                - pushes.partition_point(|&(target, _)| target < pc)
        };
        // A push of its address that no jump consumes, such as a return address, can enter it.
        let op = self.bytecode.get(pc as usize)?;
        if *op == opcode::JUMPDEST && pushes(&self.address_pushes) != pushes(&self.jumps) {
            return None;
        }
        let entries = self.fallthrough_entries(&anchors).ok()?;
        let arm_is_taken = self.then_is_taken(&anchors, arms)? ^ (path_id == 1);
        Some(if arm_is_taken {
            (taken.clone(), Some(jump as u32), entries)
        } else {
            (fallthrough.clone(), None, entries)
        })
    }

    /// Returns whether the taken side of an `if`'s jump enters its then arm, or `None` when the
    /// sides cannot be told apart.
    ///
    /// The code that each side runs alone, before it joins the other's, decides when an arm has
    /// code there, including the code of the functions that the arm calls by name, which a
    /// compiler can inline. An arm that is the jump itself is the taken side. An arm without code
    /// is entered by the side that does not reach the next statement of the block, such as the loop
    /// exit of a `break` or the increment of a `continue`, but only when the sides never meet and
    /// the other side does not reach that statement either: a side that carries a value into the
    /// join point, as with `if (c) { x = 2; }`, meets the other side's code.
    fn then_is_taken(&self, anchors: &(ItemAnchor, ItemAnchor), arms: &IfArms) -> Option<bool> {
        let (fallthrough, taken) =
            (self.trace(anchors.0.instruction), self.trace(anchors.1.instruction));
        let shared = |ic: &usize| fallthrough.contains(ic) && taken.contains(ic);
        let alone = |trace: &[usize]| trace.iter().copied().take_while(|ic| !shared(ic)).count();
        let (fallthrough_alone, taken_alone) =
            (&fallthrough[..alone(&fallthrough)], &taken[..alone(&taken)]);
        let taken_enters = |arm: &SourceLocation, callees: &[SourceLocation]| {
            let holds = |trace: &[usize]| {
                trace.iter().any(|&ic| {
                    self.starts_in_range(ic, arm)
                        || callees.iter().any(|callee| self.starts_in_range(ic, callee))
                })
            };
            match (holds(fallthrough_alone), holds(taken_alone)) {
                (false, true) => Some(true),
                (true, false) => Some(false),
                _ => None,
            }
        };
        let then_taken = taken_enters(&arms.then_arm, &arms.then_callees);
        let else_taken = arms
            .else_arm
            .as_ref()
            .and_then(|arm| taken_enters(arm, &arms.else_callees))
            .map(|taken| !taken);
        match (then_taken, else_taken) {
            (Some(then), Some(other)) if then != other => return None,
            (Some(taken), _) | (None, Some(taken)) => return Some(taken),
            (None, None) => {}
        }
        // An arm that is only a `break` or `continue` can become the jump itself.
        let is_body = |arm| self.jump_is_body(anchors, arm);
        match (is_body(&arms.then_arm), arms.else_arm.as_ref().is_some_and(is_body)) {
            (true, false) => return Some(true),
            (false, true) => return Some(false),
            (true, true) => return None,
            (false, false) => {}
        }
        let meet = fallthrough.len() > fallthrough_alone.len();
        if arms.else_arm.is_some() || self.has_own_code(&arms.then_arm) || meet {
            return None;
        }
        let after = arms.after.as_ref()?;
        let starts_after = |trace: &[usize]| {
            trace
                .iter()
                .find(|&&ic| self.bytecode[self.pcs[ic] as usize] != opcode::JUMPDEST)
                .is_some_and(|&ic| self.starts_in_range(ic, after))
        };
        let reaches_after =
            |trace: &[usize]| trace.iter().any(|&ic| self.starts_in_range(ic, after));
        match (starts_after(&fallthrough), starts_after(&taken)) {
            (true, false) if !reaches_after(&taken) => Some(true),
            (false, true) if !reaches_after(&fallthrough) => Some(false),
            _ => None,
        }
    }

    /// Returns the instructions that run from `pc` up to the first conditional jump, dynamic jump,
    /// or halt, following static `PUSH target; JUMP` hops.
    fn trace(&self, pc: u32) -> Vec<usize> {
        let mut trace = Vec::new();
        let mut next = self.pcs.binary_search(&pc).ok();
        // Bounds a cycle of static jumps.
        for _ in 0..MAX_TRACE_HOPS {
            let mut push = None;
            while let Some(ic) = next {
                let Some(&pc) = self.pcs.get(ic) else { return trace };
                trace.push(ic);
                next = Some(ic + 1);
                match self.bytecode[pc as usize] {
                    opcode::JUMP => {
                        next = push.and_then(|target| self.pcs.binary_search(&target).ok());
                        break;
                    }
                    opcode::JUMPI
                    | opcode::STOP
                    | opcode::RETURN
                    | opcode::REVERT
                    | opcode::INVALID
                    | opcode::SELFDESTRUCT => return trace,
                    op @ opcode::PUSH1..=opcode::PUSH4 => {
                        let immediate = &self.bytecode[pc as usize + 1..];
                        let len = (op - opcode::PUSH0) as usize;
                        push = jump_destination(&immediate[..len]).ok();
                    }
                    _ => push = None,
                }
            }
            if next.is_none() {
                break;
            }
        }
        trace
    }

    /// Returns whether the instruction `ic` is mapped to the byte range of `loc`, as
    /// [`Self::simple`] matches, which tolerates a statement's span ending before its `;`.
    fn starts_in_range(&self, ic: usize, loc: &SourceLocation) -> bool {
        self.source_map.get(ic).is_some_and(|element| is_in_source_range(element, loc))
    }

    /// Returns whether an instruction mapped inside `loc` runs only when `loc` does. The pushes
    /// that feed a conditional jump do not: they run with the jump, even when its target is the
    /// body of a `break` or `continue`.
    fn has_own_code(&self, loc: &SourceLocation) -> bool {
        self.instructions_starting_in(loc).any(|ic| {
            self.starts_in_range(ic, loc)
                && self.branches.binary_search_by_key(&ic, |&(ic, ..)| ic).is_err()
        })
    }

    /// With exactly one source reference to an internal function, its unique native entry
    /// identifies that direct call even when a tail transfer emitted no instruction of its own.
    fn unique_callee_entry(&self, item_id: u32, callee: &SourceLocation) -> Option<ItemAnchor> {
        let mut labels = InstIter::new(self.bytecode).with_pc().enumerate().filter_map(
            |(ic, (pc, instruction))| {
                (instruction.opcode.get() == opcode::JUMPDEST
                    && self.source_map.get(ic).is_some_and(|location| {
                        location.index() == Some(callee.source_id as u32)
                            && location.offset() == callee.bytes.start
                            && location.length() == callee.len()
                    }))
                .then_some(pc)
            },
        );
        let entry = labels.next()?;
        if labels.next().is_some() {
            return None;
        }
        Some(ItemAnchor { item_id, instruction: entry.try_into().ok()? })
    }

    /// An alias-only function has no instructions for some of its statements. Its unique normal
    /// return proves those statements completed. Require an exact declaration location, an explicit
    /// function-exit marker, and a pure return stub; never infer hits for inlined or aborting code.
    fn alias_return(&self, item_id: u32, function: &SourceLocation) -> Option<ItemAnchor> {
        let exact = |element: &SourceElement| {
            element.index() == Some(function.source_id as u32)
                && element.offset() == function.bytes.start
                && element.length() == function.len()
        };
        let mut returns = Vec::new();
        for (ic, (pc, instruction)) in InstIter::new(self.bytecode).with_pc().enumerate() {
            if instruction.opcode.get() != opcode::JUMPDEST
                || !self.source_map.get(ic).is_some_and(exact)
            {
                continue;
            }
            for (relative, (offset, instruction)) in
                InstIter::new(&self.bytecode[pc..]).with_pc().enumerate().take(32)
            {
                match instruction.opcode.get() {
                    opcode::JUMPDEST if relative == 0 => {}
                    opcode::PUSH0
                    | opcode::PUSH1..=opcode::PUSH32
                    | opcode::DUP1..=opcode::DUP16
                    | opcode::SWAP1..=opcode::SWAP16
                    | opcode::POP => {}
                    opcode::JUMP => {
                        if self
                            .source_map
                            .get(ic + relative)
                            .is_some_and(|element| exact(element) && element.jump() == Jump::Out)
                        {
                            returns.push(pc + offset);
                        }
                        break;
                    }
                    _ => break,
                }
            }
        }
        let [pc] = returns.as_slice() else { return None };
        Some(ItemAnchor { item_id, instruction: (*pc).try_into().ok()? })
    }

    /// A shared revert has no unique body location. Recover its controlling edge only when a
    /// single decision matches the condition and exactly one successor emits the resolved error.
    /// Payload matching determines polarity; source order and compiler identity do not.
    fn revert_guard(
        &self,
        item_id: u32,
        guard: &super::RevertGuard,
    ) -> Option<(ItemAnchor, Option<u32>)> {
        let loc = &guard.condition;
        let mut decisions = self.branches.iter().filter(|&&(ic, _, _)| {
            self.source_map.get(ic + 1).is_some_and(|jump| {
                jump.index() == Some(loc.source_id as u32)
                    && jump.offset() == loc.bytes.start
                    && jump.length() == loc.len()
            })
        });
        let &(_, jump_pc, immediate) = decisions.next()?;
        if decisions.next().is_some() || self.bytecode.get(jump_pc + 1) == Some(&opcode::JUMPDEST) {
            return None;
        }
        let (fallthrough, taken) = ItemAnchor::from_branch(item_id, jump_pc, immediate).ok()?;
        let rejects = |pc| constant_revert_selector(self.bytecode, pc) == Some(guard.selector);
        match (rejects(fallthrough.instruction as usize), rejects(taken.instruction as usize)) {
            (true, false) if self.continues_after(taken.instruction as usize, guard) => {
                Some((fallthrough, None))
            }
            (false, true) if self.continues_after(fallthrough.instruction as usize, guard) => {
                Some((taken, Some(jump_pc.try_into().ok()?)))
            }
            _ => None,
        }
    }

    /// The other successor must positively reach the source continuation in this same function.
    /// An unsupported error stub is unknown, not evidence for the opposite branch's polarity.
    fn continues_after(&self, start: usize, guard: &super::RevertGuard) -> bool {
        for (ic, (_, instruction)) in InstIter::new(self.bytecode)
            .with_pc()
            .enumerate()
            .skip_while(|(_, (pc, _))| *pc < start)
            .take(32)
        {
            match instruction.opcode.get() {
                opcode::JUMP
                | opcode::JUMPI
                | opcode::REVERT
                | opcode::INVALID
                | opcode::STOP
                | opcode::RETURN
                | opcode::SELFDESTRUCT
                | opcode::CALL
                | opcode::STATICCALL
                | opcode::DELEGATECALL
                | opcode::CALLCODE => return false,
                _ => {}
            }
            if self.source_map.get(ic).is_some_and(|element| {
                element.index() == Some(guard.function.source_id as u32)
                    && element.offset() >= guard.statement.bytes.end
                    && element.offset() >= guard.function.bytes.start
                    && element
                        .offset()
                        .checked_add(element.length())
                        .is_some_and(|end| end <= guard.function.bytes.end)
            }) {
                return true;
            }
        }
        false
    }
}

/// Recognizes a constant four-byte error payload in a small straight-line stub. Memory and the
/// incoming stack are unknown: only bytes explicitly stored by this stub can establish a selector.
/// Calls, branches, dynamic data, and unsupported operations deliberately leave it unrecognized.
fn constant_revert_selector(bytecode: &[u8], pc: usize) -> Option<[u8; 4]> {
    let mut stack = Vec::<U256>::new();
    let mut memory = [None; 64];
    for (_, inst) in InstIter::new(bytecode.get(pc..)?).with_pc().take(32) {
        match inst.opcode.get() {
            opcode::JUMPDEST => {}
            opcode::PUSH0 => stack.push(U256::ZERO),
            opcode::PUSH1..=opcode::PUSH32 => stack.push(U256::from_be_slice(inst.immediate)),
            opcode::DUP1..=opcode::DUP16 => {
                let depth = usize::from(inst.opcode.get() - opcode::DUP1) + 1;
                stack.push(*stack.get(stack.len().checked_sub(depth)?)?);
            }
            opcode::SWAP1..=opcode::SWAP16 => {
                let depth = usize::from(inst.opcode.get() - opcode::SWAP1) + 1;
                let top = stack.len().checked_sub(1)?;
                let other = top.checked_sub(depth)?;
                stack.swap(top, other);
            }
            opcode::SHL => {
                let shift = stack.pop()?;
                let value = stack.pop()?;
                stack.push(if shift < U256::from(256) {
                    value << shift.to::<usize>()
                } else {
                    U256::ZERO
                });
            }
            opcode::MSTORE => {
                let offset = usize::try_from(stack.pop()?).ok()?;
                let value = stack.pop()?.to_be_bytes::<32>();
                for (dest, byte) in
                    memory.get_mut(offset..offset.checked_add(32)?)?.iter_mut().zip(value)
                {
                    *dest = Some(byte);
                }
            }
            opcode::REVERT => {
                let offset = usize::try_from(stack.pop()?).ok()?;
                if stack.pop()? != U256::from(4) {
                    return None;
                }
                let bytes = memory.get(offset..offset.checked_add(4)?)?;
                return Some([bytes[0]?, bytes[1]?, bytes[2]?, bytes[3]?]);
            }
            _ => return None,
        }
    }
    None
}

/// Returns whether `instruction` is in one of the ordered, disjoint `runs`.
fn in_reverting_run(runs: &[Range<usize>], instruction: usize) -> bool {
    let next = runs.partition_point(|run| run.end <= instruction);
    runs.get(next).is_some_and(|run| run.contains(&instruction))
}

/// Calculates whether `element` is entirely inside the span of `location`.
fn is_within_source_span(element: &SourceElement, location: &SourceLocation) -> bool {
    element.index_i32() == location.source_id as i32
        && element.offset() >= location.bytes.start
        && element.offset() + element.length() <= location.bytes.start + location.len()
}

/// Calculates whether `element` is within the range of the target `location`.
fn is_in_source_range(element: &SourceElement, location: &SourceLocation) -> bool {
    // Source IDs must match.
    let source_ids_match = element.index_i32() == location.source_id as i32;
    if !source_ids_match {
        return false;
    }

    // Needed because some source ranges in the source map mark the entire contract...
    let is_within_start = element.offset() >= location.bytes.start;
    if !is_within_start {
        return false;
    }

    let start_of_ranges = location.bytes.start.max(element.offset());
    let end_of_ranges =
        (location.bytes.start + location.len()).min(element.offset() + element.length());
    start_of_ranges <= end_of_ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CoverageItem;
    use foundry_compilers::artifacts::sourcemap;

    /// Returns the `(instruction, jump)` of each anchor of the branches of the given kinds at the
    /// given source offsets, each spanning 20 bytes. Branch `n` has items `2n` and `2n + 1` for
    /// its paths 0 and 1.
    fn branch_anchors(
        bytecode: &[u8],
        starts: &[(u32, BranchKind)],
        source_map: &str,
    ) -> Vec<(u32, Option<u32>)> {
        let anchors = find_branch_anchors(bytecode, starts, source_map);
        anchors
            .anchors
            .iter()
            .enumerate()
            .map(|(index, anchor)| {
                assert_eq!(anchor.item_id as usize, index);
                (anchor.instruction, anchors.jumps.get(&index).copied())
            })
            .collect()
    }

    fn find_branch_anchors(
        bytecode: &[u8],
        starts: &[(u32, BranchKind)],
        source_map: &str,
    ) -> ItemAnchors {
        let items = starts
            .iter()
            .enumerate()
            .flat_map(|(branch_id, &(start, _))| {
                (0..2).map(move |path_id| CoverageItem {
                    kind: CoverageItemKind::Branch {
                        branch_id: branch_id as u32,
                        path_id,
                        is_first_opcode: false,
                    },
                    loc: location(start..start + 20),
                    anchor_loc: None,
                    hits: 0,
                })
            })
            .collect();
        let kinds = starts
            .iter()
            .enumerate()
            .map(|(id, (_, kind))| (id as u32, kind.clone()))
            .collect::<Vec<_>>();
        let analysis = SourceAnalysis::for_test(items, kinds);
        let source_map = sourcemap::parse(source_map).unwrap();
        let ic_pc_map = IcPcMap::new(bytecode);
        find_anchors(bytecode, &source_map, &ic_pc_map, &analysis)
    }

    fn require_anchors(
        bytecode: &[u8],
        starts: &[u32],
        source_map: &str,
    ) -> Vec<(u32, Option<u32>)> {
        let starts = starts.iter().map(|&start| (start, BranchKind::Require)).collect::<Vec<_>>();
        branch_anchors(bytecode, &starts, source_map)
    }

    #[test]
    fn require_paths_follow_the_failing_side() {
        // `cond; PUSH dest; JUMPI`, then the continuation or the revert.
        let solc = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let solar = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            6,
            opcode::JUMPI,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        // Both sides revert, so the layout is ambiguous and the fallthrough is path 0.
        let both_revert = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        // The taken side branches again before it can revert.
        let taken_branches = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            6,
            opcode::JUMPI,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            14,
            opcode::JUMPI,
            opcode::STOP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        for (bytecode, expected) in [
            (&solc[..], [(5, None), (8, Some(4))]),
            (&solar[..], [(6, Some(4)), (5, None)]),
            (&both_revert[..], [(5, None), (8, Some(4))]),
            (&taken_branches[..], [(5, None), (6, Some(4))]),
        ] {
            let count = InstIter::new(bytecode).count();
            // Only the condition and the jump belong to the require.
            let source_map = format!("10:20:0;;;50:5:0{}", ";".repeat(count - 4));
            assert_eq!(require_anchors(bytecode, &[10], &source_map), expected);
        }
    }

    #[test]
    fn require_paths_prefer_the_source_map() {
        // The success path of solc's `require(c, "m"); revert Foo();` reaches a straight-line
        // revert, while the failure code encodes the message through a dynamic jump. The failing
        // side is the one that starts inside the require.
        let solc = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            7,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::JUMP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        // Solar's cold block follows an unmapped `JUMPDEST` and is not provably reverting.
        let solar = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            6,
            opcode::JUMPI,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::JUMP,
        ];
        for (bytecode, source_map, expected) in [
            (&solc[..], "10:20:0;;;;;;50:13:0;;;", [(5, None), (7, Some(4))]),
            // Both sides start inside the span, as for `require(c, E(1)); assembly { invalid() }`.
            (&solc[..], "10:20:0;;;;;;;;;", [(5, None), (7, Some(4))]),
            // An element that starts inside the span but extends past it is not inside.
            (&solc[..], "10:20:0;;;15:40:0;;;50:13:0;;;", [(7, Some(4)), (5, None)]),
            (&solar[..], "10:20:0;;;50:5:0;-1:-1:-1;10:20:0;", [(6, Some(4)), (5, None)]),
        ] {
            assert_eq!(require_anchors(bytecode, &[10], source_map), expected);
        }
    }

    #[test]
    fn yul_if_paths_follow_the_body() {
        // `cond; PUSH dest; JUMPI`, then the body or the code after the `if`. The `if` spans bytes
        // 10..30 and its body 20..30.
        let body_falls_through = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            7,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let body_is_taken = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            6,
            opcode::JUMPI,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::POP,
        ];
        let kind = BranchKind::YulIf(SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: 20..30,
            lines: 1..2,
        });
        for (bytecode, source_map, expected) in [
            (&body_falls_through[..], "10:20:0;;;22:5:0;;50:5:0;", [(5, None), (7, Some(4))]),
            // Solar can place the body at the jump target, behind an unmapped `JUMPDEST`.
            (&body_is_taken[..], "10:20:0;;;50:5:0;-1:-1:-1;22:5:0;", [(6, Some(4)), (5, None)]),
            // Without a single side in the body, path 0 is the fallthrough.
            (&body_falls_through[..], "10:20:0;;;50:5:0;;50:5:0;", [(5, None), (7, Some(4))]),
            (&body_falls_through[..], "10:20:0;;;22:5:0;;22:5:0;", [(5, None), (7, Some(4))]),
            (&body_is_taken[..], "10:20:0;;;50:5:0;-1:-1:-1;50:5:0;", [(5, None), (6, Some(4))]),
        ] {
            assert_eq!(branch_anchors(bytecode, &[(10, kind.clone())], source_map), expected);
        }
    }

    #[test]
    fn yul_if_jump_only_body_is_the_taken_side() {
        // A body that is only a `continue` becomes the conditional jump itself, so neither side
        // starts in the body. The `if` spans bytes 10..30 and its body 22..30.
        let kind = BranchKind::YulIf(SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: 22..30,
            lines: 1..2,
        });
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            7,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        for (source_map, expected) in [
            // The jump's push is mapped to the `continue`, inside the body.
            ("12:5:0;25:2:0;12:5:0;50:5:0;;50:5:0;", [(7, Some(4)), (5, None)]),
            // The push is mapped to the condition, so the positional default stays.
            ("12:5:0;;;50:5:0;;50:5:0;", [(5, None), (7, Some(4))]),
        ] {
            assert_eq!(branch_anchors(&bytecode, &[(10, kind.clone())], source_map), expected);
        }
    }

    #[test]
    fn yul_if_takes_its_own_jump() {
        // An outer `if` whose body holds another conditional jump. The `if` spans bytes 10..30 and
        // its body 22..30. The outer jump is at pc 4 and the nested one at pc 9.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            12,
            opcode::JUMPI,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            11,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::JUMPDEST,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let kind = BranchKind::YulIf(SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: 22..30,
            lines: 1..2,
        });
        for (source_map, expected) in [
            // The nested jump is mapped inside the body, so the condition's jump is the `if`'s.
            ("12:5:0;;;25:2:0;;;26:3:0;;50:5:0;", [(5, None), (12, Some(4))]),
            // Without a jump outside the body, the last jump is the `if`'s.
            ("22:5:0;;;25:2:0;;;26:3:0;;50:5:0;", [(10, None), (11, Some(9))]),
        ] {
            assert_eq!(branch_anchors(&bytecode, &[(10, kind.clone())], source_map), expected);
        }
    }

    #[test]
    fn yul_if_counts_entries_into_a_shared_fallthrough() {
        // An `if` whose body starts at a loop head. The `if` spans bytes 10..30 and its body
        // 22..30. The jump at pc 10 either returns to the head, which the `if` falls into, or
        // leaves the loop.
        let kind = BranchKind::YulIf(SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: 22..30,
            lines: 1..2,
        });
        let bytecode = |back_edge, jump| {
            [
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                11,
                opcode::JUMPI,
                opcode::JUMPDEST,
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                back_edge,
                jump,
                opcode::JUMPDEST,
                opcode::STOP,
            ]
        };
        let source_map = "12:5:0;;;25:2:0;;;;;;50:5:0;";
        let find =
            |bytecode: [u8; 13]| find_branch_anchors(&bytecode, &[(10, kind.clone())], source_map);
        // The back edge's executions are not the `if`'s, and both paths discount them.
        let anchors = find(bytecode(5, opcode::JUMP));
        assert_eq!(
            anchors.anchors.iter().map(|anchor| anchor.instruction).collect::<Vec<_>>(),
            [5, 11]
        );
        assert_eq!(anchors.jumps.get(&1), Some(&4));
        assert_eq!(anchors.entries.get(&0), Some(&vec![(10, false)]));
        assert_eq!(anchors.entries.get(&1), Some(&vec![(10, false)]));
        // A conditional jump back to the head counts its taken executions, unless its own
        // fallthrough is a `JUMPDEST`, which others can enter.
        let anchors = find(bytecode(5, opcode::JUMPI));
        assert_eq!(anchors.anchors.len(), 2);
        assert!(anchors.entries.is_empty());
        let conditional = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            12,
            opcode::JUMPI,
            opcode::JUMPDEST,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            5,
            opcode::JUMPI,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let anchors = find_branch_anchors(&conditional, &[(10, kind.clone())], source_map);
        assert_eq!(anchors.entries.get(&0), Some(&vec![(10, true)]));
        // A jump elsewhere leaves the fallthrough unshared.
        let anchors = find(bytecode(11, opcode::JUMP));
        assert_eq!(anchors.anchors.len(), 2);
        assert!(anchors.entries.is_empty());
    }

    #[test]
    fn yul_if_body_at_the_jump_target_with_a_shared_skip_label() {
        // Solar's layout of `if c { ...; if d { break } continue }`. The body is the jump target,
        // behind a `JUMPDEST` mapped to the whole `if` (bytes 10..30, its body 22..30). The skip
        // side is an unmapped `JUMPDEST` that the body's own conditional jump also targets.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            9,
            opcode::JUMPI,
            opcode::JUMPDEST,
            opcode::PUSH1,
            0,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::DUP1,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            5,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::STOP,
        ];
        let kind = BranchKind::YulIf(SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: 22..30,
            lines: 1..2,
        });
        let source_map = "12:5:0;;;-1:-1:-1;50:5:0;;10:20:0;25:2:0;;;;;";
        let anchors = find_branch_anchors(&bytecode, &[(10, kind)], source_map);
        // Path 0 is the jump target, which counts the jump's executions minus the skip side's,
        // and the skip side's hits discount the entries of the body's own jump.
        assert_eq!(
            anchors.anchors.iter().map(|anchor| anchor.instruction).collect::<Vec<_>>(),
            [9, 5]
        );
        assert_eq!(anchors.jumps.get(&0), Some(&4));
        assert_eq!(anchors.entries.get(&0), Some(&vec![(15, true)]));
        assert_eq!(anchors.entries.get(&1), Some(&vec![(15, true)]));
    }

    #[test]
    fn requires_share_a_revert_block() {
        // Both requires jump to the same revert block, the first through a `PUSH; JUMP` stub.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            15,
            opcode::JUMPI,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            19,
            opcode::JUMPI,
            opcode::STOP,
            opcode::INVALID,
            opcode::INVALID,
            opcode::INVALID,
            opcode::INVALID,
            opcode::JUMPDEST,
            opcode::PUSH1,
            19,
            opcode::JUMP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        // Each require maps its condition and jump. Other code lies outside both spans.
        let source_map = format!(
            "10:20:0;;;40:20:0;;;90:5:0{}",
            ";".repeat(InstIter::new(&bytecode).count() - 7)
        );
        assert_eq!(
            require_anchors(&bytecode, &[10, 40], &source_map),
            [(15, Some(4)), (5, None), (19, Some(9)), (10, None)]
        );
    }

    fn location(bytes: Range<u32>) -> SourceLocation {
        SourceLocation { source_id: 0, contract_name: "T".into(), bytes, lines: 1..2 }
    }

    /// Returns the `(instruction, jump)` of the anchors for the two paths of a ternary spanning
    /// bytes 10..50, with its true arm at 20..30 and its false arm at 35..45.
    fn ternary_anchors(bytecode: &[u8], source_map: &str) -> Vec<(u32, Option<u32>)> {
        let items = (0..2)
            .map(|path_id| CoverageItem {
                kind: CoverageItemKind::Branch { branch_id: 0, path_id, is_first_opcode: false },
                loc: location(10..50),
                anchor_loc: None,
                hits: 0,
            })
            .collect();
        let arms = TernaryArms { true_arm: location(20..30), false_arm: location(35..45) };
        let analysis = SourceAnalysis::for_test(items, vec![(0, BranchKind::Ternary(arms))]);
        let source_map = sourcemap::parse(source_map).unwrap();
        let ic_pc_map = IcPcMap::new(bytecode);
        let anchors = find_anchors(bytecode, &source_map, &ic_pc_map, &analysis);
        (0..anchors.anchors.len())
            .map(|i| (anchors.anchors[i].instruction, anchors.jumps.get(&i).copied()))
            .collect()
    }

    #[test]
    fn ternary_paths_follow_the_arms() {
        // `cond; PUSH dest; JUMPI`, then the code of the fallthrough arm, then `dest`, which holds
        // the code of the jump target arm.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH1,
            0,
            opcode::POP,
            opcode::STOP,
        ];
        // Path 0 is the false arm and path 1 the true arm. `(pc, jump)` is the fallthrough for
        // `(5, None)` and the jump target for `(8, Some(4))`.
        let false_falls_through = [(5, None), (8, Some(4))];
        let true_falls_through = [(8, Some(4)), (5, None)];
        // The jump `10:40` is mapped exactly to the ternary, and the cond to `12:3`.
        // The arms' first instructions map to the enclosing `0:100`, as with solc via-IR, which
        // jumps over the arms' setup code: only a later instruction maps inside the arm.
        for (source_map, expected) in [
            // The true arm is the jump target, as with solc's legacy pipeline.
            ("12:3:0;;10:40:0;36:3:0;;0:100:0;-1:-1:-1;22:3:0;;", false_falls_through),
            // The true arm falls through.
            ("12:3:0;;10:40:0;22:3:0;;0:100:0;-1:-1:-1;36:3:0;;", true_falls_through),
            // Only the entry code of an arm maps inside it, after a first instruction outside it.
            ("12:3:0;;10:40:0;0:100:0;22:3:0;0:100:0;-1:-1:-1;36:3:0;;", true_falls_through),
            // The false arm has no code, as with a literal already on the stack: the fallthrough
            // reaches the join point, so the true arm is the jump target.
            ("12:3:0;;10:40:0;0:100:0;;0:100:0;-1:-1:-1;22:3:0;;", false_falls_through),
            // The same polarity, with the code of the arm that has code on the other side.
            ("12:3:0;;10:40:0;22:3:0;;0:100:0;-1:-1:-1;0:100:0;;", true_falls_through),
            // The true arm has no code and the false arm is the jump target.
            ("12:3:0;;10:40:0;0:100:0;;0:100:0;-1:-1:-1;36:3:0;;", true_falls_through),
            // The true arm has no code and the false arm falls through.
            ("12:3:0;;10:40:0;36:3:0;;0:100:0;-1:-1:-1;0:100:0;;", false_falls_through),
            // Neither arm has code, so the positional default stays.
            ("12:3:0;;10:40:0;0:100:0;;0:100:0;-1:-1:-1;0:100:0;;", false_falls_through),
            // Both successors map inside the same arm, so the positional default stays.
            ("12:3:0;;10:40:0;22:3:0;;0:100:0;-1:-1:-1;22:3:0;;", false_falls_through),
        ] {
            assert_eq!(ternary_anchors(&bytecode, source_map), expected, "{source_map}");
        }
    }

    #[test]
    fn ternary_anchor_rejects_missing_node_mapping() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        let bytecode = [opcode::PUSH1, 6, opcode::JUMPI, opcode::PUSH1, 7, opcode::JUMPI];
        // A contained inner span must never substitute for the missing outer decision.
        let inner_only = sourcemap::parse("15:5:0;;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &inner_only, &ic_pc_map);
        assert!(index.branch(0, &loc, true, None).is_err());
    }

    fn source_location(start: u32, len: u32) -> SourceLocation {
        SourceLocation {
            source_id: 0,
            contract_name: "T".into(),
            bytes: start..start + len,
            lines: 1..2,
        }
    }

    fn constant_error_stub() -> Vec<u8> {
        vec![
            opcode::PUSH4,
            0xe3,
            0x37,
            0x2e,
            0x2d,
            opcode::PUSH0,
            opcode::MSTORE,
            opcode::PUSH1,
            4,
            opcode::PUSH1,
            28,
            opcode::REVERT,
        ]
    }

    #[test]
    fn shared_revert_guards_use_their_own_incoming_edge() {
        let mut bytecode = vec![
            opcode::PUSH0,
            opcode::ISZERO,
            opcode::PUSH1,
            12,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::ISZERO,
            opcode::PUSH1,
            12,
            opcode::JUMPI,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
        ];
        bytecode.extend(constant_error_stub());
        let text = InstIter::new(&bytecode)
            .enumerate()
            .map(|(ic, _)| match ic {
                0..=3 => "10:10:0",
                4..=7 => "30:10:0",
                8 => "50:10:0",
                _ => "-1:-1:-1",
            })
            .collect::<Vec<_>>()
            .join(";");
        let source_map = sourcemap::parse(&text).unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let mut anchors = ItemAnchors::default();
        for (item_id, start) in [(0, 10), (1, 30)] {
            let guard = super::super::RevertGuard {
                condition: source_location(start, 10),
                statement: source_location(start, 10),
                function: source_location(0, 100),
                selector: [0xe3, 0x37, 0x2e, 0x2d],
            };
            let (anchor, jump) = index.revert_guard(item_id, &guard).unwrap();
            anchors.jumps.insert(anchors.anchors.len(), jump.unwrap());
            anchors.anchors.push(anchor);
        }
        let ambiguous_text = InstIter::new(&bytecode)
            .enumerate()
            .map(|(ic, _)| if ic < 8 { "10:10:0" } else { "-1:-1:-1" })
            .collect::<Vec<_>>()
            .join(";");
        let ambiguous_map = sourcemap::parse(&ambiguous_text).unwrap();
        let ambiguous = AnchorIndex::new(&bytecode, &ambiguous_map, &ic_pc_map);
        let guard = super::super::RevertGuard {
            condition: source_location(10, 10),
            statement: source_location(10, 10),
            function: source_location(0, 100),
            selector: [0xe3, 0x37, 0x2e, 0x2d],
        };
        assert!(ambiguous.revert_guard(0, &guard).is_none());
        let mut hit_map = super::super::HitMap::new(alloy_primitives::Bytes::new());
        hit_map.hits(4, 1);
        hit_map.hits(9, 1);
        hit_map.hits(10, 1);
        hit_map.hits(12, 1);
        let hits = anchors
            .hits(&hit_map)
            .map(|(anchor, hits)| (anchor.item_id, hits.get()))
            .collect::<Vec<_>>();
        assert_eq!(hits, vec![(0, 1)], "the shared destination must not cover the other guard");
    }

    #[test]
    fn revert_guard_rejects_ambiguous_payloads_and_decisions() {
        let mut bytecode = vec![opcode::PUSH0, opcode::PUSH1, 16, opcode::JUMPI];
        bytecode.extend(constant_error_stub());
        bytecode.push(opcode::JUMPDEST);
        bytecode.extend(constant_error_stub());
        let text = InstIter::new(&bytecode)
            .enumerate()
            .map(|(ic, _)| if ic < 3 { "10:10:0" } else { "-1:-1:-1" })
            .collect::<Vec<_>>()
            .join(";");
        let source_map = sourcemap::parse(&text).unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let guard = super::super::RevertGuard {
            condition: source_location(10, 10),
            statement: source_location(10, 10),
            function: source_location(0, 100),
            selector: [0xe3, 0x37, 0x2e, 0x2d],
        };
        assert!(index.revert_guard(0, &guard).is_none());
        assert!(
            constant_revert_selector(&[opcode::PUSH1, 4, opcode::PUSH0, opcode::REVERT], 0)
                .is_none(),
            "uninitialized memory is unknown"
        );
    }

    #[test]
    fn revert_guard_does_not_treat_unknown_as_the_opposite_path() {
        let mut bytecode = vec![
            opcode::PUSH0,
            opcode::PUSH1,
            19,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::MLOAD,
            opcode::POP,
        ];
        bytecode.extend(constant_error_stub());
        bytecode.push(opcode::JUMPDEST);
        bytecode.extend(constant_error_stub());
        let text = InstIter::new(&bytecode)
            .enumerate()
            .map(|(ic, _)| if ic < 3 { "10:10:0" } else { "-1:-1:-1" })
            .collect::<Vec<_>>()
            .join(";");
        let source_map = sourcemap::parse(&text).unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let guard = super::super::RevertGuard {
            condition: source_location(10, 10),
            statement: source_location(10, 40),
            function: source_location(0, 100),
            selector: [0xe3, 0x37, 0x2e, 0x2d],
        };
        assert!(index.revert_guard(0, &guard).is_none());
    }

    #[test]
    fn erased_call_requires_one_exact_callee_entry() {
        let bytecode = [opcode::JUMPDEST, opcode::STOP, opcode::JUMPDEST, opcode::STOP];
        let ic_pc_map = IcPcMap::new(&bytecode);
        for (map, expected) in
            [("10:20:0;;-1:-1:-1;", Some(0)), ("10:20:0;;;", None), ("11:19:0;;-1:-1:-1;", None)]
        {
            let source_map = sourcemap::parse(map).unwrap();
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(
                index
                    .unique_callee_entry(0, &source_location(10, 20))
                    .map(|anchor| anchor.instruction),
                expected
            );
        }
    }

    #[test]
    fn alias_statements_require_a_unique_normal_return() {
        let bytecode = [opcode::JUMPDEST, opcode::PUSH1, 42, opcode::SWAP1, opcode::JUMP];
        let source_map = sourcemap::parse("10:20:0;;;10:20:0:o").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let anchor = index.alias_return(0, &source_location(10, 20)).unwrap();
        assert_eq!(anchor.instruction, 4);
        let anchors = ItemAnchors { anchors: vec![anchor], ..Default::default() };
        let uncalled = super::super::HitMap::new(alloy_primitives::Bytes::new());
        assert_eq!(anchors.hits(&uncalled).count(), 0);
        for unsupported in
            [opcode::MLOAD, opcode::JUMPI, opcode::CALL, opcode::INVALID, opcode::REVERT]
        {
            let bytecode = [opcode::JUMPDEST, unsupported, opcode::SWAP1, opcode::JUMP];
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert!(index.alias_return(0, &source_location(10, 20)).is_none());
        }
    }

    #[test]
    fn statement_anchors_do_not_depend_on_cold_stub_order() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        for (bytecode, expected) in [
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::RETURN,
                ],
                3,
            ),
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::RETURN,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                ],
                0,
            ),
            (vec![opcode::PUSH0, opcode::PUSH0, opcode::INVALID], 0),
            (vec![opcode::PUSH0, opcode::PUSH0, opcode::REVERT], 0),
            (
                vec![
                    opcode::PUSH0,
                    opcode::PUSH1,
                    7,
                    opcode::JUMPI,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::STOP,
                ],
                0,
            ),
            // A JUMPDEST-led failure stub before the real code is skipped.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::RETURN,
                ],
                4,
            ),
            // Code falling through into the stub's JUMPDEST is not part of the failure run.
            (
                vec![
                    opcode::PUSH0,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::STOP,
                ],
                0,
            ),
            // Adjacent JUMPDESTs lead the same failure stub.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::STOP,
                ],
                5,
            ),
            // A JUMPDEST-led failure stub is still the fallback when it is the only candidate.
            (vec![opcode::JUMPDEST, opcode::PUSH0, opcode::PUSH0, opcode::REVERT], 0),
        ] {
            let count = InstIter::new(&bytecode).count();
            let source_map =
                sourcemap::parse(&format!("10:20:0{}", ";".repeat(count - 1))).unwrap();
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.simple(0, &loc).unwrap().instruction, expected);
        }
    }

    #[test]
    fn statement_anchor_skips_blocks_reached_only_from_inside() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        // Code outside the span (`0:100`) jumps over a block that belongs to the span, and which
        // is laid out before the span's entry.
        for (bytecode, source_map, expected) in [
            // The entry falls through from outside, and the earlier block is only jumped to from
            // inside.
            (
                vec![
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;;0:100:0;12:3:0;",
                6,
            ),
            // A return address pushed from inside, before a call, does not make the earlier
            // block an entry.
            (
                vec![
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;;0:100:0;12:3:0;;;",
                6,
            ),
            // A constant that equals a `JUMPDEST` address, but is not a jump or return address, is
            // not a way in: the dynamically entered block stays the anchor.
            (
                vec![
                    opcode::PUSH0,
                    opcode::CALLDATALOAD,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::PUSH0,
                    opcode::MSTORE,
                    opcode::PUSH1,
                    0x20,
                    opcode::PUSH0,
                    opcode::RETURN,
                    opcode::JUMPDEST,
                    opcode::STOP,
                ],
                "0:100:0;;;15:5:0;;;;;;;;;;",
                3,
            ),
            // The entry is jumped to from outside.
            (
                vec![
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;;16:4:0;12:3:0;",
                5,
            ),
            // An unmapped `JUMPDEST` belongs to the block it leads, so it does not make the code
            // after it an entry.
            (
                vec![
                    opcode::PUSH1,
                    7,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;-1:-1:-1;15:5:0;;16:4:0;12:3:0;",
                7,
            ),
            // A jump mapped to a longer span that starts inside the item, as the parent
            // expression's merge label does, does not make the earlier block an entry.
            (
                vec![
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;10:50:0;;16:4:0;10:50:0;",
                5,
            ),
            // A block that is also entered from outside, as a loop head is, stays an entry.
            (
                vec![opcode::JUMPDEST, opcode::JUMPDEST, opcode::PUSH1, 1, opcode::JUMP],
                "0:100:0;12:3:0;15:2:0;;",
                1,
            ),
            (
                vec![
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::STOP,
                    opcode::PUSH1,
                    3,
                    opcode::STOP,
                ],
                "0:100:0;;15:5:0;;12:3:0;;;0:100:0;;",
                3,
            ),
            // A push from outside to an unmapped `JUMPDEST` makes the block it leads an entry.
            (
                vec![
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;-1:-1:-1;12:3:0;;",
                5,
            ),
            // A block that nothing is known to reach is an entry even when a later block is
            // entered from outside.
            (
                vec![
                    opcode::PUSH1,
                    7,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "0:100:0;;15:5:0;;16:4:0;;17:3:0;;",
                5,
            ),
            // The lowest instruction stays the anchor when nothing is known to reach it, as with
            // a function entered by a dynamic jump.
            (
                vec![opcode::STOP, opcode::JUMPDEST, opcode::PUSH0, opcode::STOP],
                "0:100:0;15:5:0;;",
                1,
            ),
        ] {
            let source_map = sourcemap::parse(source_map).unwrap();
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.simple(0, &loc).unwrap().instruction, expected, "{source_map:?}");
        }
    }

    #[test]
    fn function_anchor_prefers_jumped_to_entry_over_earlier_body_block() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        // A body block (inner span) is laid out before the entry JUMPDEST (exact span), which a
        // call site jumps to.
        let bytecode = [
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
            opcode::PUSH1,
            3,
            opcode::JUMP,
        ];
        let ic_pc_map = IcPcMap::new(&bytecode);
        for (source_map, expected) in [
            // Call site outside the function.
            ("15:5:0;;;10:20:0;;0:100:0;", 3),
            // A same-start but shorter span is not the entry.
            ("15:5:0;;;10:5:0;;0:100:0;", 0),
            // The only jump is inside the function, so the JUMPDEST is its exit.
            ("15:5:0;;;10:20:0;;12:3:0;", 0),
        ] {
            let source_map = sourcemap::parse(source_map).unwrap();
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.simple(0, &loc).unwrap().instruction, 0);
            assert_eq!(index.function(0, &loc).unwrap().instruction, expected, "{source_map:?}");
        }

        // A lower exact-span `JUMPDEST` that nothing jumps to, such as a via-IR return point, and
        // an exact-span failure stub that only the function itself jumps to are not the entry.
        for (bytecode, source_map, expected) in [
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "10:20:0;;;;;;0:100:0;",
                3,
            ),
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMPI,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                ],
                "10:20:0;;;;;;12:3:0;;0:100:0;",
                4,
            ),
            // A public function's ABI wrapper (jumped to by the dispatcher) falls into its body
            // (jumped to by internal calls), which is the anchor that runs on both paths.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::POP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    3,
                    opcode::JUMP,
                ],
                "10:20:0;;;;;;0:100:0;;;",
                3,
            ),
            // A `JUMPI` to a failure stub does not end the run, so the wrapper still falls into
            // the body.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    13,
                    opcode::JUMPI,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                ],
                "10:20:0;;;;;;0:100:0;;;;10:20:0;;;",
                4,
            ),
            // A `JUMPI` to other code ends the run, as with `&&` and `||` join blocks, so the
            // lowest candidate does not fall into a tail shared with other code.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    4,
                    opcode::JUMPI,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                ],
                "10:20:0;;;;;;0:100:0;;;",
                0,
            ),
            // A `JUMPI` to a block that jumps to a failure stub, such as an optimized panic helper
            // call, does not end the run either.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    13,
                    opcode::JUMPI,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    17,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::PUSH0,
                    opcode::REVERT,
                ],
                "10:20:0;;;;;;0:100:0;;;;-1:-1:-1;;;;;;",
                4,
            ),
            // An unmapped helper `JUMP`, such as an ABI decoder returning to the wrapper, does not
            // end the run, but a mapped `JUMP` does.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                ],
                "10:20:0;;-1:-1:-1;10:20:0;;;0:100:0;;;",
                4,
            ),
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    4,
                    opcode::JUMP,
                ],
                "10:20:0;;12:3:0;10:20:0;;;0:100:0;;;",
                0,
            ),
            // An exact-span block that other code jumps to, as an optimizer-merged exit, is not
            // preferred when no other candidate falls through into it.
            (
                vec![
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::JUMP,
                    opcode::JUMPDEST,
                    opcode::PUSH0,
                    opcode::JUMPDEST,
                    opcode::STOP,
                    opcode::PUSH1,
                    0,
                    opcode::JUMP,
                    opcode::PUSH1,
                    5,
                    opcode::JUMP,
                ],
                "10:20:0;;;12:3:0;;10:20:0;;0:100:0;;;",
                0,
            ),
        ] {
            let source_map = sourcemap::parse(source_map).unwrap();
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(index.function(0, &loc).unwrap().instruction, expected);
        }

        // An exact-span instruction that nothing jumps to is not an entry, as in an inlined
        // constructor whose exact span only marks its trailing cleanup.
        let bytecode = [opcode::PUSH0, opcode::POP, opcode::PUSH0, opcode::POP, opcode::STOP];
        let source_map = sourcemap::parse("15:5:0;;10:20:0;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        assert_eq!(index.function(0, &loc).unwrap().instruction, 0);
    }

    /// Returns the `(instruction, jump)` of the anchors for the paths of an `if` spanning bytes
    /// 10..70, with its then arm at 40..60 and, if given, its else arm at 61..69.
    fn if_anchors(bytecode: &[u8], source_map: &str, else_arm: bool) -> Vec<(u32, Option<u32>)> {
        let anchors = find_if_anchors(bytecode, source_map, else_arm);
        (0..anchors.anchors.len())
            .map(|i| (anchors.anchors[i].instruction, anchors.jumps.get(&i).copied()))
            .collect()
    }

    fn find_if_anchors(bytecode: &[u8], source_map: &str, else_arm: bool) -> ItemAnchors {
        find_if_anchors_after(bytecode, source_map, else_arm, true)
    }

    /// Like [`find_if_anchors`], with the next statement at 80..90 only if `after`.
    fn find_if_anchors_after(
        bytecode: &[u8],
        source_map: &str,
        else_arm: bool,
        after: bool,
    ) -> ItemAnchors {
        find_if_anchors_calling(bytecode, source_map, else_arm, after, None, None)
    }

    /// Like [`find_if_anchors_after`], with the function that each arm calls at the given byte
    /// range.
    fn find_if_anchors_calling(
        bytecode: &[u8],
        source_map: &str,
        else_arm: bool,
        after: bool,
        then_callee: Option<Range<u32>>,
        else_callee: Option<Range<u32>>,
    ) -> ItemAnchors {
        let arms = IfArms {
            then_arm: location(40..60),
            else_arm: else_arm.then(|| location(61..69)),
            after: after.then(|| location(80..90)),
            then_callees: then_callee.into_iter().map(location).collect(),
            else_callees: else_callee.into_iter().map(location).collect(),
        };
        let items = (0..2 - u32::from(!else_arm))
            .map(|path_id| CoverageItem {
                kind: CoverageItemKind::Branch { branch_id: 0, path_id, is_first_opcode: true },
                loc: location(10..70),
                anchor_loc: Some(if path_id == 0 {
                    arms.then_arm.clone()
                } else {
                    arms.else_arm.clone().unwrap()
                }),
                hits: 0,
            })
            .collect();
        let analysis = SourceAnalysis::for_test(items, vec![(0, BranchKind::If(arms))]);
        let source_map = sourcemap::parse(source_map).unwrap();
        let ic_pc_map = IcPcMap::new(bytecode);
        find_anchors(bytecode, &source_map, &ic_pc_map, &analysis)
    }

    #[test]
    fn if_arms_without_code_count_the_jump_edge() {
        // `cond; PUSH exit; JUMPI`, then the code after the `if` and, at `exit`, an epilogue. The
        // `if` is mapped to 10..70, its condition to 12..15, the code after it to 80..85, and the
        // epilogue to the enclosing function, as with a `break` that leaves a loop.
        let after_then_exit = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::RETURN,
        ];
        // The same code with the epilogue first, as when the compiler jumps over it.
        let exit_then_after = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::RETURN,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
        ];
        // The arm is the taken edge: its count is the jump's minus the fallthrough's.
        let taken = [(8, Some(4))];
        // The arm is the fallthrough edge: its count is the hits of the instruction after the jump.
        let fallthrough = [(5, None)];
        for (bytecode, source_map, expected) in [
            (&after_then_exit[..], "12:3:0;;10:60:0;80:5:0;;;0:100:0;;", &taken[..]),
            // A jump target push mapped inside the arm runs with the jump, so it is not the arm's.
            (&after_then_exit[..], "12:3:0;45:5:0;10:60:0;80:5:0;;;0:100:0;;", &taken),
            (&exit_then_after[..], "12:3:0;;10:60:0;0:100:0;;;80:5:0;;", &fallthrough),
            // The jump's element can run one byte past the span of an `if` without braces, which
            // ends before the `;`.
            (&after_then_exit[..], "12:3:0;;10:61:0;80:5:0;;;0:100:0;;", &taken),
            // The arm continues a loop at code mapped before the `if`.
            (&after_then_exit[..], "12:3:0;;10:60:0;80:5:0;;;5:3:0;;", &taken),
            // The push is the arm even when both sides reach code after the `if`.
            (&after_then_exit[..], "12:3:0;45:5:0;10:60:0;80:5:0;;;85:5:0;;", &taken),
            // Both sides reach code outside the `if`, so nothing tells the arm from the skip.
            (&after_then_exit[..], "12:3:0;;10:60:0;80:5:0;;;85:5:0;;", &[]),
            // The arm has code of its own, which anchors it as before.
            (&exit_then_after[..], "12:3:0;;10:60:0;45:5:0;;;80:5:0;;", &fallthrough),
        ] {
            assert_eq!(if_anchors(bytecode, source_map, false), expected, "{source_map}");
        }
    }

    #[test]
    fn if_edges_follow_the_arm_with_code() {
        // An else arm with code tells the sides apart, so the then arm without code is the other.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
        ];
        // Path 0 counts the jump's edge and path 1 anchors on the code of its own.
        assert_eq!(
            if_anchors(&bytecode, "12:3:0;;10:60:0;63:3:0;;;80:5:0;;", true),
            [(8, Some(4)), (5, None)]
        );
        // The else arm is the taken side, so path 0 is the fallthrough.
        assert_eq!(
            if_anchors(&bytecode, "12:3:0;;10:60:0;80:5:0;;;63:3:0;;", true),
            [(5, None), (8, None)]
        );
        // An arm with code on each side decides nothing alone, so nothing is guessed for it.
        assert_eq!(if_anchors(&bytecode, "12:3:0;;10:60:0;63:3:0;;;63:3:0;;", true), [(5, None)]);
    }

    #[test]
    fn if_edges_follow_the_code_of_called_functions() {
        // Neither arm has code of its own: each calls a function that the compiler inlined, so
        // the source map attributes its code to the function. The then arm calls the function at
        // 100..110 and the else arm the one at 120..130.
        let bytecode = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
        ];
        // Each flag makes the arm call the function at 100..110 for the then arm and 120..130 for
        // the else arm.
        let anchors = |source_map, then_calls: bool, else_calls: bool| {
            let anchors = find_if_anchors_calling(
                &bytecode,
                source_map,
                true,
                true,
                then_calls.then_some(100..110),
                else_calls.then_some(120..130),
            );
            (0..anchors.anchors.len())
                .map(|i| (anchors.anchors[i].instruction, anchors.jumps.get(&i).copied()))
                .collect::<Vec<_>>()
        };
        // The taken side runs the then arm's function, so path 0 counts the jump's edge and path 1
        // the fallthrough, however the two are laid out.
        assert_eq!(
            anchors("12:3:0;;10:60:0;125:3:0;;;105:3:0;;;", true, true),
            [(8, Some(4)), (5, None)]
        );
        assert_eq!(
            anchors("12:3:0;;10:60:0;105:3:0;;;125:3:0;;;", true, true),
            [(5, None), (8, Some(4))]
        );
        // One arm's function is enough to tell the sides apart.
        assert_eq!(
            anchors("12:3:0;;10:60:0;125:3:0;;;105:3:0;;;", false, true),
            [(8, Some(4)), (5, None)]
        );
        // Both sides run the same function, so nothing is guessed.
        assert!(anchors("12:3:0;;10:60:0;105:3:0;;;105:3:0;;;", true, false).is_empty());
        // Neither arm calls a function that the code runs.
        assert!(anchors("12:3:0;;10:60:0;125:3:0;;;105:3:0;;;", false, false).is_empty());
        assert!(anchors("12:3:0;;10:60:0;225:3:0;;;205:3:0;;;", true, true).is_empty());
    }

    #[test]
    fn if_edges_are_not_guessed_when_ambiguous() {
        // Without a next statement in the block, as when the `if` ends a loop body, the skip side
        // is not known to reach any code, so a `break` is told apart by its jump target only.
        let after_then_exit = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            8,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::RETURN,
        ];
        let anchors = find_if_anchors_after(
            &after_then_exit,
            "12:3:0;;10:60:0;80:5:0;;;0:100:0;;",
            false,
            false,
        );
        assert!(anchors.anchors.is_empty());

        // The skip side runs code that is mapped to the function, and joins the other side's
        // code, as with `if (c) { x = 2; }` when the constant feeds the join point.
        let joined = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            6,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::STOP,
        ];
        assert!(if_anchors(&joined, "12:3:0;;10:60:0;0:100:0;10:60:0;80:5:0;;", false).is_empty());

        // The fallthrough is a `JUMPDEST` that other code can enter. A static jump to it is
        // discounted from its hits, but a push of its address that no jump consumes, such as a
        // return address, may enter it any number of times.
        let entered = |entry: &[u8]| {
            let mut bytecode = vec![
                opcode::PUSH1,
                0,
                opcode::PUSH1,
                8,
                opcode::JUMPI,
                opcode::JUMPDEST,
                opcode::PUSH0,
                opcode::RETURN,
                opcode::JUMPDEST,
                opcode::PUSH0,
                opcode::STOP,
            ];
            bytecode.extend_from_slice(entry);
            let source_map =
                format!("12:3:0;;10:60:0;0:100:0;;;80:5:0;;;{}", ";".repeat(entry.len()));
            let anchors = find_if_anchors(&bytecode, &source_map, false);
            (anchors.anchors.len(), anchors.entries.get(&0).cloned())
        };
        assert_eq!(entered(&[]), (1, None));
        // `PUSH1 5; JUMP` at pc 11 and 13.
        assert_eq!(entered(&[opcode::PUSH1, 5, opcode::JUMP]), (1, Some(vec![(13, false)])));
        assert_eq!(entered(&[opcode::PUSH1, 5, opcode::PUSH1, 0, opcode::JUMP]), (0, None));
        // A constant that no jump follows is not an entry.
        assert_eq!(entered(&[opcode::PUSH1, 5, opcode::POP]), (1, None));

        // A second jump inside the `if` and outside its arms, as in `if (a && b)`, may enter the
        // arm too.
        let and = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            13,
            opcode::JUMPI,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            13,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::RETURN,
        ];
        assert!(if_anchors(&and, "12:3:0;;12:3:0;;;10:60:0;80:5:0;;;0:100:0;;", false).is_empty());

        // A jump to a failure in the condition, such as an overflow check, cannot enter the arm.
        let checked = [
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            16,
            opcode::JUMPI,
            opcode::PUSH1,
            0,
            opcode::PUSH1,
            13,
            opcode::JUMPI,
            opcode::PUSH0,
            opcode::POP,
            opcode::STOP,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::RETURN,
            opcode::JUMPDEST,
            opcode::PUSH0,
            opcode::PUSH0,
            opcode::REVERT,
        ];
        assert_eq!(
            if_anchors(&checked, "12:3:0;;12:3:0;;;10:60:0;80:5:0;;;0:100:0;;;0:100:0;;;", false),
            [(13, Some(9))]
        );
    }
}
