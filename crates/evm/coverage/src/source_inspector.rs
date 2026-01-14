use crate::{HitMap, HitMaps};
use alloy_primitives::{Address, Bytes, B256, U256};
use revm::{
    context::ContextTr,
    inspector::JournalExt,
    interpreter::{CallInputs, CallOutcome, Interpreter, InterpreterResult, InstructionResult},
    Inspector,
};
use std::ptr::NonNull;

/// Address of the specialized cheatcode contract for coverage.
/// address(uint160(uint256(keccak256("hevm cheat code"))))
pub const CHEATCODE_ADDRESS: Address = Address::new([
    0x71, 0x09, 0x70, 0x9E, 0xCf, 0xa9, 0x1a, 0x80, 0x62, 0x6f,
    0xF3, 0x98, 0x9D, 0x68, 0xf6, 0x7F, 0x5b, 0x1D, 0xD1, 0x2D,
]);

/// Selector for `coverageHit(uint256,uint256)`: `0xa46d5036`
pub const COVERAGE_HIT_SELECTOR: u32 = 0xa46d5036;

#[derive(Debug)]
pub struct SourceCoverageCollector {
    current_map: NonNull<HitMap>,
    current_hash: B256,
    pub maps: HitMaps,
}

impl Clone for SourceCoverageCollector {
    fn clone(&self) -> Self {
        Self {
            current_map: NonNull::dangling(),
            current_hash: B256::ZERO,
            maps: self.maps.clone(),
        }
    }
}

impl Default for SourceCoverageCollector {
    fn default() -> Self {
        Self {
            current_map: NonNull::dangling(),
            current_hash: B256::ZERO,
            maps: Default::default(),
        }
    }
}

// SAFETY: See comments on `current_map` in `LineCoverageCollector`.
unsafe impl Send for SourceCoverageCollector {}
unsafe impl Sync for SourceCoverageCollector {}

impl<CTX> Inspector<CTX> for SourceCoverageCollector
where
    CTX: ContextTr<Journal: JournalExt>,
{
    fn initialize_interp(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        self.get_or_insert_map(interpreter);
    }

    fn step(&mut self, interpreter: &mut Interpreter, _context: &mut CTX) {
        self.get_or_insert_map(interpreter);
    }

    fn call(
        &mut self,
        context: &mut CTX,
        inputs: &mut CallInputs,
    ) -> Option<CallOutcome> {
        if inputs.target_address == CHEATCODE_ADDRESS {
            let input_bytes = inputs.input.bytes(context);
            if input_bytes.len() >= 4 {
                let selector = u32::from_be_bytes(input_bytes[0..4].try_into().unwrap());
                if selector == COVERAGE_HIT_SELECTOR {
                    if input_bytes.len() >= 68 {
                        // let source_id_uint = U256::from_be_slice(&input_bytes[4..36]);
                        let item_id_uint = U256::from_be_slice(&input_bytes[36..68]);
                        
                        // Cast to u32/usize. Coverage IDs are usize.
                        let id = item_id_uint.to::<u32>(); 
                        
                        // The hit is attributed to the CURRENT contract which is executing.
                        // In `call`, the `interpreter` context from `step` belongs to the CALLER.
                        // So `self.current_map` should point to the caller's HitMap.
                        
                        unsafe {
                            if self.current_map != NonNull::dangling() {
                                 self.current_map.as_mut().hit(id);
                            }
                        }
                    }
                    
                    return Some(CallOutcome {
                        result: InterpreterResult {
                            result: InstructionResult::Return,
                            output: Bytes::new(),
                            gas: revm::interpreter::Gas::new(0),
                        },
                        memory_offset: inputs.return_memory_offset.clone(),
                        was_precompile_called: false,
                        precompile_call_logs: vec![],
                    });
                }
            }
        }
        
        None
    }
}

impl SourceCoverageCollector {
    /// Finish collecting coverage information and return the [`HitMaps`].
    pub fn finish(self) -> HitMaps {
        self.maps
    }

    /// Gets the hit map for the current contract, or inserts a new one if it doesn't exist.
    #[inline]
    fn get_or_insert_map(&mut self, interpreter: &mut Interpreter) -> &mut HitMap {
        // We use get_or_calculate_hash because we need the hash to key the map.
        // Source coverage relies on bytecode hash to map back to source via source maps (or we might need to change that later).
        // For now, we assume we can map bytecode hash -> source info.
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
            .or_insert_with(|| HitMap::new(interpreter.bytecode.original_bytes()))
            .into();
    }
}
