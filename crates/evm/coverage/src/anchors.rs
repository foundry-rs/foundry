use super::{
    CoverageItemKind, ExecutionAnchor, ExecutionAnchorKind, ItemAnchor, ItemAnchors, SourceLocation,
};
use crate::analysis::{EmptySpecialFunctionKind, SourceAnalysis};
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
        // We do not support program counters bigger than u32.
        ensure!(immediate.len() <= 4, "jump destination overflow");
        let mut pc_bytes = [0u8; 4];
        pc_bytes[4 - immediate.len()..].copy_from_slice(immediate);
        let pc_jump = u32::from_be_bytes(pc_bytes);
        Ok((
            Self { item_id, instruction: (next_pc + 1).try_into()? },
            Self { item_id, instruction: pc_jump },
        ))
    }
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
    let select_branch = |(fallthrough, taken): (ItemAnchor, ItemAnchor), path_id| match path_id {
        0 => (fallthrough, None),
        1 => (taken, Some(fallthrough.instruction - 1)),
        _ => panic!("too many path IDs for branch"),
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
            match item.kind {
                CoverageItemKind::Branch { path_id: 1, is_first_opcode: true, .. }
                    if item.anchor_loc.is_some() =>
                {
                    index.simple(item_id, anchor_loc).map(|anchor| (anchor, None)).or_else(|_| {
                        index
                            .branch(item_id, &item.loc, false)
                            .map(|anchors| select_branch(anchors, 1))
                    })
                }
                CoverageItemKind::Branch { branch_id, path_id, is_first_opcode: false } => {
                    let exact = analysis.is_ternary_branch(item.loc.source_id as u32, branch_id);
                    index
                        .branch(item_id, anchor_loc, exact)
                        .map(|anchors| select_branch(anchors, path_id))
                }
                _ => index.simple(item_id, anchor_loc).map(|anchor| (anchor, None)),
            }
            .or_else(|error| {
                let Some(hints) = analysis.anchor_hints(item_id) else { return Err(error) };
                if let Some(guard) = &hints.revert_guard
                    && let Some(anchor) = index.revert_guard(item_id, guard)
                {
                    return Ok(anchor);
                }
                if let Some(function) = &hints.alias_function
                    && let Some(anchor) = index.alias_return(item_id, function)
                {
                    return Ok((anchor, None));
                }
                if let Some(callee) = &hints.unique_callee
                    && let Some(anchor) = index.unique_callee_entry(item_id, callee)
                {
                    return Ok((anchor, None));
                }
                Err(error)
            })
            .inspect_err(|err| warn!(%item, %err, "could not find anchor"))
            .ok()
        })
        .for_each(|(anchor, jump)| {
            if let Some(jump) = jump {
                anchors.jumps.insert(anchors.anchors.len(), jump);
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

/// Source map and branch lookups precomputed once per bytecode.
struct AnchorIndex<'a> {
    bytecode: &'a [u8],
    source_map: &'a SourceMap,
    ic_pc_map: &'a IcPcMap,
    /// `(source id, offset, instruction)`, sorted for source range lookups.
    entries: Vec<(i32, u32, usize)>,
    /// `(ic, next pc, push immediate)` for every push that feeds a `JUMPI`.
    branches: Vec<(usize, usize, &'a [u8])>,
    /// Straight-line instruction ranges ending in REVERT or INVALID, ordered by start.
    reverting_runs: Vec<Range<usize>>,
}

impl<'a> AnchorIndex<'a> {
    fn new(bytecode: &'a [u8], source_map: &'a SourceMap, ic_pc_map: &'a IcPcMap) -> Self {
        let mut entries = source_map
            .iter()
            .enumerate()
            .map(|(instruction, element)| (element.index_i32(), element.offset(), instruction))
            .collect::<Vec<_>>();
        entries.sort_unstable();
        let mut branches = Vec::new();
        let mut reverting_runs = Vec::new();
        let mut run_start = 0;
        let mut prev = opcode::STOP;
        for (ic, (pc, inst)) in InstIter::new(bytecode).with_pc().enumerate() {
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
                if bytecode.get(next_pc).copied() == Some(opcode::JUMPI) {
                    branches.push((ic, next_pc, inst.immediate));
                }
            }
        }
        Self { bytecode, source_map, ic_pc_map, entries, branches, reverting_runs }
    }

    /// Finds the first opcode in the source range, preferring code outside a failure stub.
    fn simple(&self, item_id: u32, loc: &SourceLocation) -> eyre::Result<ItemAnchor> {
        let source = loc.source_id as i32;
        let start = self
            .entries
            .partition_point(|&(id, offset, _)| (id, offset) < (source, loc.bytes.start));
        let end = self.entries.partition_point(|&(id, offset, _)| {
            (id, offset) <= (source, loc.bytes.start + loc.len())
        });
        let candidates = self.entries[start..end]
            .iter()
            .map(|&(_, _, instruction)| instruction)
            .filter(|&instruction| is_in_source_range(&self.source_map[instruction], loc));
        // Cold validation stubs can precede the statement's main code in the bytecode.
        // A revert statement itself may have only failure-path candidates, so retain the fallback.
        let instruction = candidates
            .clone()
            .filter(|&instruction| {
                let next = self.reverting_runs.partition_point(|run| run.end <= instruction);
                !self.reverting_runs.get(next).is_some_and(|run| run.contains(&instruction))
            })
            .min()
            .or_else(|| candidates.min());
        ItemAnchor::from_instruction(instruction, self.ic_pc_map, item_id, loc)
    }

    /// Finds the fallthrough and taken anchors of a branch.
    ///
    /// `exact` matches only the jump mapped to exactly `loc`, so ternaries skip nested decisions.
    fn branch(
        &self,
        item_id: u32,
        loc: &SourceLocation,
        exact: bool,
    ) -> eyre::Result<(ItemAnchor, ItemAnchor)> {
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
                anchors = Some(found);
            }
        }
        anchors.ok_or_else(|| eyre::eyre!("Could not detect branches in source: {}", loc))
    }

    /// Recover an erased call only at an exact declaration label with a proven incoming edge
    /// from outside the callee. Return labels with only internal predecessors are completions,
    /// not entries. A real fallthrough edge also qualifies: a tail call can emit no jump.
    fn unique_callee_entry(&self, item_id: u32, callee: &SourceLocation) -> Option<ItemAnchor> {
        let mut incoming = FxHashSet::default();
        let mut previous_push = None;
        for (ic, (pc, instruction)) in InstIter::new(self.bytecode).with_pc().enumerate() {
            let outside = self.source_map.get(ic).is_some_and(|location| {
                location.jump() != Jump::Out
                    && location.length() != 0
                    && location.offset().checked_add(location.length()).is_some_and(|end| {
                        location.index().is_some_and(|source| {
                            source != callee.source_id as u32
                                || end <= callee.bytes.start
                                || location.offset() >= callee.bytes.end
                        })
                    })
            });
            let opcode = instruction.opcode.get();
            if outside {
                if matches!(opcode, opcode::JUMP | opcode::JUMPI)
                    && let Some(immediate) = previous_push
                    && let Ok((_, target)) = ItemAnchor::from_branch(item_id, pc, immediate)
                {
                    incoming.insert(target.instruction as usize);
                }
                if !matches!(
                    opcode,
                    opcode::JUMP
                        | opcode::STOP
                        | opcode::RETURN
                        | opcode::REVERT
                        | opcode::INVALID
                        | opcode::SELFDESTRUCT
                ) {
                    incoming.insert(pc + 1 + instruction.immediate.len());
                }
            }
            previous_push =
                (opcode::PUSH1..=opcode::PUSH32).contains(&opcode).then_some(instruction.immediate);
        }
        let mut labels = InstIter::new(self.bytecode).with_pc().enumerate().filter_map(
            |(ic, (pc, instruction))| {
                (instruction.opcode.get() == opcode::JUMPDEST
                    && incoming.contains(&pc)
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
    use foundry_compilers::artifacts::sourcemap;

    #[test]
    fn ternary_anchor_rejects_missing_node_mapping() {
        let loc =
            SourceLocation { source_id: 0, contract_name: "T".into(), bytes: 10..30, lines: 1..2 };
        let bytecode = [opcode::PUSH1, 6, opcode::JUMPI, opcode::PUSH1, 7, opcode::JUMPI];
        // A contained inner span must never substitute for the missing outer decision.
        let inner_only = sourcemap::parse("15:5:0;;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &inner_only, &ic_pc_map);
        assert!(index.branch(0, &loc, true).is_err());
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
    fn erased_call_distinguishes_entries_from_completions() {
        let bytecode = [
            opcode::PUSH1,
            4,
            opcode::JUMP,
            opcode::INVALID,
            opcode::JUMPDEST,
            opcode::PUSH1,
            8,
            opcode::JUMP,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let maps = [
            // Both labels have the function span, but only pc 4 is entered from the caller.
            ("100:10:0;;-1:-1:-1;10:20:0;12:1:0;;10:20:0;12:1:0", Some(4)),
            // An exit label alone cannot stand in for an entry, even if it has the exact span.
            ("100:10:0;;-1:-1:-1;12:1:0;;;10:20:0;12:1:0", None),
            // A return from a helper outside this function is still a completion edge.
            ("100:10:0;100:10:0:o;-1:-1:-1:-;10:20:0;12:1:0;;10:20:0;12:1:0", None),
            // Unknown or overlapping source ranges do not prove an external predecessor.
            ("-1:-1:-1;;;10:20:0;12:1:0;;10:20:0;12:1:0", None),
            ("0:200:0;;-1:-1:-1;10:20:0;12:1:0;;10:20:0;12:1:0", None),
            ("0:0:1;;-1:-1:-1;10:20:0;12:1:0;;10:20:0;12:1:0", None),
            // Native helpers can be called from another source file in the same frame.
            ("100:10:1;;-1:-1:-1;10:20:0;12:1:0;;10:20:0;12:1:0", Some(4)),
        ];
        for (map, expected) in maps {
            let source_map = sourcemap::parse(map).unwrap();
            let ic_pc_map = IcPcMap::new(&bytecode);
            let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
            assert_eq!(
                index.unique_callee_entry(0, &source_location(10, 20)).map(|a| a.instruction),
                expected,
                "{map}"
            );
        }
    }

    #[test]
    fn erased_tail_call_can_fall_through_into_a_reverting_callee() {
        let bytecode =
            [opcode::PUSH1, 0, opcode::JUMPDEST, opcode::PUSH0, opcode::PUSH0, opcode::REVERT];
        let source_map = sourcemap::parse("100:10:0;10:20:0;12:1:0;;;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let anchor = index.unique_callee_entry(0, &source_location(10, 20)).unwrap();
        assert_eq!(anchor.instruction, 2);
    }

    #[test]
    fn erased_call_rejects_two_externally_entered_labels() {
        let bytecode = [
            opcode::PUSH1,
            4,
            opcode::JUMP,
            opcode::INVALID,
            opcode::JUMPDEST,
            opcode::STOP,
            opcode::PUSH1,
            10,
            opcode::JUMP,
            opcode::INVALID,
            opcode::JUMPDEST,
            opcode::STOP,
        ];
        let source_map =
            sourcemap::parse("100:10:0;;-1:-1:-1;10:20:0;;100:10:0;;-1:-1:-1;10:20:0;").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        assert!(index.unique_callee_entry(0, &source_location(10, 20)).is_none());
    }

    #[test]
    fn alias_statements_require_a_unique_normal_return() {
        let bytecode = [opcode::JUMPDEST, opcode::PUSH1, 42, opcode::SWAP1, opcode::JUMP];
        let source_map = sourcemap::parse("10:20:0;;;10:20:0:o").unwrap();
        let ic_pc_map = IcPcMap::new(&bytecode);
        let index = AnchorIndex::new(&bytecode, &source_map, &ic_pc_map);
        let anchor = index.alias_return(0, &source_location(10, 20)).unwrap();
        assert_eq!(anchor.instruction, 4);
        let anchors = ItemAnchors { anchors: vec![anchor], jumps: Default::default() };
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
}
