use crate::{CallData, HitMap, HitMaps};
use alloy_primitives::{B256, map::B256HashMap};
use revm::{
    Inspector,
    interpreter::{CreateInputs, CreateOutcome, Interpreter, interpreter_types::Jumps},
};
use std::ptr::NonNull;

/// Inspector implementation for collecting coverage information.
#[derive(Clone, Debug)]
pub struct LineCoverageCollector {
    // NOTE: `current_map` is always a valid reference into `maps`.
    // It is accessed only through `get_or_insert_map` which guarantees that it's valid.
    // Both of these fields are unsafe to access directly outside of `*insert_map`.
    current_map: NonNull<CollectedHitMap>,
    current_hash: B256,

    maps: B256HashMap<CollectedHitMap>,
}

/// A [`HitMap`] being collected.
///
/// Instruction hits are counted in a dense array indexed by program counter, which avoids a hash
/// map update on every step. [`LineCoverageCollector::finish`] moves the hit program counters
/// into the [`HitMap`].
#[derive(Clone, Debug)]
struct CollectedHitMap {
    map: HitMap,
    hits: Vec<u32>,
    /// Program counters with a non-zero hit count, in order of their first hit.
    hit_pcs: Vec<u32>,
}

impl CollectedHitMap {
    #[inline]
    fn hit(&mut self, pc: usize) {
        if let Some(hits) = self.hits.get_mut(pc) {
            if *hits == 0 {
                self.hit_pcs.push(pc as u32);
            }
            *hits += 1;
        } else {
            self.grow_and_hit(pc);
        }
    }

    /// Counts a hit past the end of the padded bytecode.
    #[cold]
    #[inline(never)]
    fn grow_and_hit(&mut self, pc: usize) {
        self.hits.resize(pc + 1, 0);
        self.hit(pc);
    }

    fn finish(self) -> HitMap {
        let Self { mut map, hits, hit_pcs } = self;
        map.reserve(hit_pcs.len());
        for pc in hit_pcs {
            map.hits(pc, hits[pc as usize]);
        }
        map
    }
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

impl<CTX> Inspector<CTX> for LineCoverageCollector {
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        let call = interpreter.input.bytecode_address.is_some().then(|| {
            let calldata = if interpreter.input.input.is_empty() {
                CallData::Empty
            } else {
                let input = interpreter.input.input.as_bytes_memory(&interpreter.memory);
                CallData::new(&input)
            };
            (calldata, !interpreter.input.call_value.is_zero())
        });
        let map = self.get_or_insert_map(interpreter);
        if let Some((call, with_value)) = call {
            map.map.call(call, with_value);
        }
    }

    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        let pc = interpreter.bytecode.pc();
        self.get_or_insert_map(interpreter).hit(pc);
    }

    fn create_end(
        &mut self,
        _context: &mut CTX,
        inputs: &CreateInputs,
        outcome: &mut CreateOutcome,
    ) {
        if outcome.result.result.is_ok()
            && let Some(map) = self.maps.get_mut(&inputs.init_code_hash())
        {
            map.map.creation();
        }
    }
}

impl LineCoverageCollector {
    /// Finish collecting coverage information and return the [`HitMaps`].
    pub fn finish(self) -> HitMaps {
        HitMaps(self.maps.into_iter().map(|(hash, map)| (hash, map.finish())).collect())
    }

    /// Gets the hit map for the current contract, or inserts a new one if it doesn't exist.
    ///
    /// The map is stored in `current_map` and returned as a mutable reference.
    /// See comments on `current_map` for more details.
    #[inline]
    fn get_or_insert_map(&mut self, interpreter: &mut Interpreter) -> &mut CollectedHitMap {
        let hash = interpreter.bytecode.get_or_calculate_hash();
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
            .or_insert_with(|| CollectedHitMap {
                map: HitMap::new(interpreter.bytecode.original_bytes()),
                hits: vec![0; interpreter.bytecode.bytes_slice().len()],
                hit_pcs: Vec::new(),
            })
            .into();
    }
}
