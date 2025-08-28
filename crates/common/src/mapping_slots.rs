use alloy_primitives::{B256, map::B256HashMap};

/// Recorded mapping slots.
#[derive(Clone, Debug, Default)]
pub struct MappingSlots {
    /// Holds mapping parent (slots => slots)
    pub parent_slots: B256HashMap<B256>,

    /// Holds mapping key (slots => key)
    pub keys: B256HashMap<B256>,

    /// Holds mapping child (slots => slots[])
    pub children: B256HashMap<Vec<B256>>,

    /// Holds the last sha3 result `sha3_result => (data_low, data_high)`, this would only record
    /// when sha3 is called with `size == 0x40`, and the lower 256 bits would be stored in
    /// `data_low`, higher 256 bits in `data_high`.
    /// This is needed for mapping_key detect if the slot is for some mapping and record that.
    pub seen_sha3: B256HashMap<(B256, B256)>,
}

impl MappingSlots {
    /// Tries to insert a mapping slot. Returns true if it was inserted.
    pub fn insert(&mut self, slot: B256) -> bool {
        match self.seen_sha3.get(&slot).copied() {
            Some((key, parent)) => {
                if self.keys.insert(slot, key).is_some() {
                    return false;
                }
                self.parent_slots.insert(slot, parent);
                self.children.entry(parent).or_default().push(slot);
                self.insert(parent);
                true
            }
            None => false,
        }
    }
}
