//! Storage access helpers for debugger TUI views and commands.

use crate::DebugNode;
use alloy_primitives::{U256, map::IndexMap};
use revm::{bytecode::opcode, interpreter::InstructionResult};
use revm_inspectors::tracing::types::{CallTraceStep, StorageChangeReason};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageAccessKind {
    Load,
    Store,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StorageSpace {
    Persistent,
    Transient,
}

impl StorageSpace {
    pub(super) const fn noun(self) -> &'static str {
        match self {
            Self::Persistent => "storage",
            Self::Transient => "transient storage",
        }
    }

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::Persistent => "Storage",
            Self::Transient => "Transient storage",
        }
    }

    pub(super) const fn command(self) -> &'static str {
        match self {
            Self::Persistent => "storage",
            Self::Transient => "transient",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StorageAccess {
    step_index: usize,
    pc: usize,
    space: StorageSpace,
    kind: StorageAccessKind,
    slot: U256,
    value: U256,
    previous: Option<U256>,
}

impl StorageAccess {
    pub(super) const fn step_index(self) -> usize {
        self.step_index
    }

    pub(super) const fn pc(self) -> usize {
        self.pc
    }

    pub(super) const fn slot(self) -> U256 {
        self.slot
    }

    pub(super) const fn space(self) -> StorageSpace {
        self.space
    }

    pub(super) const fn value(self) -> U256 {
        self.value
    }

    pub(super) const fn op(self) -> &'static str {
        match (self.space, self.kind) {
            (StorageSpace::Persistent, StorageAccessKind::Load) => "SLOAD",
            (StorageSpace::Persistent, StorageAccessKind::Store) => "SSTORE",
            (StorageSpace::Transient, StorageAccessKind::Load) => "TLOAD",
            (StorageSpace::Transient, StorageAccessKind::Store) => "TSTORE",
        }
    }

    pub(super) fn describe(self) -> String {
        let op = self.op();
        let space = self.space.noun();

        match (self.kind, self.previous) {
            (StorageAccessKind::Store, Some(previous)) => format!(
                "{space} {op} slot {}: {} -> {}",
                hex_u256(self.slot),
                hex_u256(previous),
                hex_u256(self.value)
            ),
            _ => format!("{space} {op} slot {} = {}", hex_u256(self.slot), hex_u256(self.value)),
        }
    }
}

pub(super) fn storage_accesses_until(
    arena: &[DebugNode],
    current_node_index: usize,
    current_step_index: usize,
    space: StorageSpace,
) -> IndexMap<U256, StorageAccess> {
    let current_node = &arena[current_node_index];
    let current_absolute_step = current_node.step_offset.saturating_add(current_step_index);
    let mut accesses = IndexMap::default();

    for node in arena.iter().filter(|node| node.trace_node_idx == current_node.trace_node_idx) {
        for (step_index, _) in node.steps.iter().enumerate() {
            if node.step_offset.saturating_add(step_index) > current_absolute_step {
                break;
            }
            if let Some(access) = storage_access_at(&node.steps, step_index)
                && access.space() == space
            {
                accesses.insert(access.slot(), access);
            }
        }
    }

    accesses
}

pub(super) fn storage_access_at(
    steps: &[CallTraceStep],
    step_index: usize,
) -> Option<StorageAccess> {
    let step = steps.get(step_index)?;
    if matches!(step.op.get(), opcode::SSTORE | opcode::TSTORE)
        && !step.status.is_none_or(InstructionResult::is_ok)
    {
        return None;
    }

    if let Some(change) = step.storage_change.as_deref() {
        let kind = match change.reason {
            StorageChangeReason::SLOAD => StorageAccessKind::Load,
            StorageChangeReason::SSTORE => StorageAccessKind::Store,
        };
        return Some(StorageAccess {
            step_index,
            pc: step.pc,
            space: StorageSpace::Persistent,
            kind,
            slot: change.key,
            value: change.value,
            previous: change.had_value,
        });
    }

    let (space, kind) = match step.op.get() {
        opcode::SLOAD => (StorageSpace::Persistent, StorageAccessKind::Load),
        opcode::SSTORE => (StorageSpace::Persistent, StorageAccessKind::Store),
        opcode::TLOAD => (StorageSpace::Transient, StorageAccessKind::Load),
        opcode::TSTORE => (StorageSpace::Transient, StorageAccessKind::Store),
        _ => return None,
    };

    if kind == StorageAccessKind::Load {
        return Some(StorageAccess {
            step_index,
            pc: step.pc,
            space,
            kind,
            slot: step.stack.as_deref()?.last().copied()?,
            value: steps.get(step_index.checked_add(1)?)?.stack.as_deref()?.last().copied()?,
            previous: None,
        });
    }

    let mut stack = step.stack.as_deref()?.iter().rev();
    let slot = stack.next().copied()?;
    let value = stack.next().copied()?;
    Some(StorageAccess { step_index, pc: step.pc, space, kind, slot, value, previous: None })
}

pub(super) fn hex_u256(value: U256) -> String {
    format!("{value:#x}")
}
