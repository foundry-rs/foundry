use crate::{HitMap, HitMaps};
use alloy_primitives::B256;
use revm::{
    Inspector,
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{Interpreter, interpreter_types::Jumps},
};
use std::ptr::NonNull;

/// Inspector implementation for collecting coverage information.
#[derive(Clone, Debug)]
pub struct LineCoverageCollector {
    // NOTE: `current_map` is always a valid reference into `maps`.
    // It is accessed only through `get_or_insert_map` which guarantees that it's valid.
    // Both of these fields are unsafe to access directly outside of `*insert_map`.
    current_map: NonNull<HitMap>,
    current_hash: B256,

    maps: HitMaps,
}

// SAFETY: See comments on `current_map`.
unsafe impl Send for LineCoverageCollector {}
unsafe impl Sync for LineCoverageCollector {}

impl Default for LineCoverageCollector {
    fn default() -> Self {
        Self {
            current_map: NonNull::dangling(),
            current_hash: B256::ZERO,
            maps: Default::default(),
        }
    }
}

impl<CTX> Inspector<CTX> for LineCoverageCollector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        get_or_insert_contract_hash(interpreter);
        self.insert_map(interpreter);
    }

    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        let map = self.get_or_insert_map(interpreter);
        map.hit(interpreter.bytecode.pc() as u32);
    }
}

impl LineCoverageCollector {
    /// Finish collecting coverage information and return the [`HitMaps`].
    pub fn finish(self) -> HitMaps {
        self.maps
    }

    /// Gets the hit map for the current contract, or inserts a new one if it doesn't exist.
    ///
    /// The map is stored in `current_map` and returned as a mutable reference.
    /// See comments on `current_map` for more details.
    #[inline]
    fn get_or_insert_map(&mut self, interpreter: &mut Interpreter) -> &mut HitMap {
        let hash = get_or_insert_contract_hash(interpreter);
        if self.current_hash != *hash {
            self.insert_map(interpreter);
        }
        // SAFETY: See comments on `current_map`.
        unsafe { self.current_map.as_mut() }
    }

    #[cold]
    #[inline(never)]
    fn insert_map(&mut self, interpreter: &mut Interpreter) {
        let hash = interpreter.bytecode.hash().unwrap();
        self.current_hash = hash;
        // Converts the mutable reference to a `NonNull` pointer.
        self.current_map = self
            .maps
            .entry(hash)
            .or_insert_with(|| HitMap::new(interpreter.bytecode.original_bytes()))
            .into();
    }
}

/// Helper function for extracting contract hash used to record coverage hit map.
///
/// If the contract hash is zero (contract not yet created but it's going to be created in current
/// tx) then the hash is calculated from the bytecode.
#[inline]
fn get_or_insert_contract_hash(interpreter: &mut Interpreter) -> B256 {
    // TODO: use just `get_or_calculate_hash`
    interpreter
        .bytecode
        .hash()
        .filter(|h| !h.is_zero())
        .unwrap_or_else(|| interpreter.bytecode.regenerate_hash())
}
