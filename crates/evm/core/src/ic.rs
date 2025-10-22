use crate::bytecode::InstIter;
use alloy_primitives::map::rustc_hash::FxHashMap;
use serde::Serialize;

/// Maps from program counter to instruction counter.
///
/// Inverse of [`IcPcMap`].
#[derive(Debug, Clone, Serialize)]
#[serde(transparent)]
pub struct PcIcMap {
    inner: FxHashMap<u32, u32>,
}

impl PcIcMap {
    /// Creates a new `PcIcMap` for the given code.
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<true>(code) }
    }

    /// Returns the length of the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the instruction counter for the given program counter.
    pub fn get(&self, pc: u32) -> Option<u32> {
        self.inner.get(&pc).copied()
    }
}

/// Map from instruction counter to program counter.
///
/// Inverse of [`PcIcMap`].
pub struct IcPcMap {
    inner: FxHashMap<u32, u32>,
}

impl IcPcMap {
    /// Creates a new `IcPcMap` for the given code.
    pub fn new(code: &[u8]) -> Self {
        Self { inner: make_map::<false>(code) }
    }

    /// Returns the length of the map.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if the map is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Returns the program counter for the given instruction counter.
    pub fn get(&self, ic: u32) -> Option<u32> {
        self.inner.get(&ic).copied()
    }

    /// Iterate over the IC-PC pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&u32, &u32)> {
        self.inner.iter()
    }
}

fn make_map<const PC_FIRST: bool>(code: &[u8]) -> FxHashMap<u32, u32> {
    assert!(code.len() <= u32::MAX as usize, "bytecode is too big");
    let mut map = FxHashMap::with_capacity_and_hasher(code.len(), Default::default());
    for (ic, (pc, _)) in InstIter::new(code).with_pc().enumerate() {
        if PC_FIRST {
            map.insert(pc as u32, ic as u32);
        } else {
            map.insert(ic as u32, pc as u32);
        }
    }
    map.shrink_to_fit();
    map
}
