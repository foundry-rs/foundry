//! Storage access helpers for debugger TUI views and commands.

use alloy_primitives::U256;
use revm::bytecode::opcode;
use revm_inspectors::tracing::types::{CallTraceStep, StorageChangeReason};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StorageAccessKind {
    Sload,
    Sstore,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StorageAccess {
    step_index: usize,
    pc: usize,
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

    pub(super) fn describe(self) -> String {
        let op = match self.kind {
            StorageAccessKind::Sload => "SLOAD",
            StorageAccessKind::Sstore => "SSTORE",
        };

        match (self.kind, self.previous) {
            (StorageAccessKind::Sstore, Some(previous)) => format!(
                "storage {op} slot {}: {} -> {}",
                hex_u256(self.slot),
                hex_u256(previous),
                hex_u256(self.value)
            ),
            _ => format!("storage {op} slot {} = {}", hex_u256(self.slot), hex_u256(self.value)),
        }
    }
}

pub(super) fn find_storage_access(
    steps: &[CallTraceStep],
    current_step: usize,
    slot: U256,
) -> Option<StorageAccess> {
    if steps.is_empty() {
        return None;
    }

    let current = current_step.min(steps.len() - 1);
    storage_access_at(steps, current).filter(|access| access.slot == slot).or_else(|| {
        (current.saturating_add(1)..steps.len())
            .find_map(|i| storage_access_at(steps, i).filter(|access| access.slot == slot))
            .or_else(|| {
                (0..current)
                    .rev()
                    .find_map(|i| storage_access_at(steps, i).filter(|access| access.slot == slot))
            })
    })
}

pub(super) fn storage_access_at(
    steps: &[CallTraceStep],
    step_index: usize,
) -> Option<StorageAccess> {
    let step = steps.get(step_index)?;
    if let Some(change) = step.storage_change.as_deref() {
        let kind = match change.reason {
            StorageChangeReason::SLOAD => StorageAccessKind::Sload,
            StorageChangeReason::SSTORE => StorageAccessKind::Sstore,
        };
        return Some(StorageAccess {
            step_index,
            pc: step.pc,
            kind,
            slot: change.key,
            value: change.value,
            previous: change.had_value,
        });
    }

    if step.op.get() == opcode::SLOAD {
        return Some(StorageAccess {
            step_index,
            pc: step.pc,
            kind: StorageAccessKind::Sload,
            slot: step.stack.as_deref()?.last().copied()?,
            value: steps.get(step_index.checked_add(1)?)?.stack.as_deref()?.last().copied()?,
            previous: None,
        });
    }

    None
}

pub(super) fn hex_u256(value: U256) -> String {
    format!("{value:#x}")
}
