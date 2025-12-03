//! Storage slot identification and decoding utilities for Solidity storage layouts.
//!
//! This module provides functionality to identify and decode storage slots based on
//! Solidity storage layout information from the compiler.

use crate::mapping_slots::MappingSlots;
use alloy_dyn_abi::{DynSolType, DynSolValue};
use alloy_primitives::{B256, U256, hex, keccak256, map::B256Map};
use foundry_common_fmt::format_token_raw;
use foundry_compilers::artifacts::{Storage, StorageLayout, StorageType};
use serde::Serialize;
use std::{collections::BTreeMap, str::FromStr, sync::Arc};
use tracing::trace;

/// "inplace" encoding type for variables that fit in one storage slot i.e 32 bytes
pub const ENCODING_INPLACE: &str = "inplace";
/// "mapping" encoding type for Solidity mappings, which use keccak256 hash-based storage
pub const ENCODING_MAPPING: &str = "mapping";
/// "bytes" encoding type for bytes and string types, which use either inplace or keccak256
/// hash-based storage depending on length
pub const ENCODING_BYTES: &str = "bytes";
/// "dynamic_array" encoding type for dynamic arrays, which uses keccak256 hash-based storage
pub const ENCODING_DYN_ARRAY: &str = "dynamic_array";

/// Information about a storage slot including its label, type, and decoded values.
#[derive(Serialize, Debug)]
pub struct SlotInfo {
    /// The variable name from the storage layout.
    ///
    /// For top-level variables: just the variable name (e.g., "myVariable")
    /// For struct members: dotted path (e.g., "myStruct.memberName")
    /// For array elements: name with indices (e.g., "myArray\[0\]", "matrix\[1\]\[2\]")
    /// For nested structures: full path (e.g., "outer.inner.field")
    /// For mappings: base name with keys (e.g., "balances\[0x1234...\]")/ex
    pub label: String,
    /// The Solidity type information
    #[serde(rename = "type", serialize_with = "serialize_slot_type")]
    pub slot_type: StorageTypeInfo,
    /// Offset within the storage slot (for packed storage)
    pub offset: i64,
    /// The storage slot number as a string
    pub slot: String,
    /// For struct members, contains nested SlotInfo for each member
    ///
    /// This is populated when a struct's members / fields are packed in a single slot.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<Self>>,
    /// Decoded values (if available) - used for struct members
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded: Option<DecodedSlotValues>,
    /// Decoded mapping keys (serialized as "key" for single, "keys" for multiple)
    #[serde(
        skip_serializing_if = "Option::is_none",
        flatten,
        serialize_with = "serialize_mapping_keys"
    )]
    pub keys: Option<Vec<String>>,
}

/// Wrapper type that holds both the original type label and the parsed DynSolType.
///
/// We need both because:
/// - `label`: Used for serialization to ensure output matches user expectations
/// - `dyn_sol_type`: The parsed type used for actual value decoding
#[derive(Debug)]
pub struct StorageTypeInfo {
    /// The original type label from storage layout (e.g., "uint256", "address", "mapping(address
    /// => uint256)")
    pub label: String,
    /// The parsed dynamic Solidity type used for decoding
    pub dyn_sol_type: DynSolType,
}

impl SlotInfo {
    /// Decodes a single storage value based on the slot's type information.
    ///
    /// Note: For decoding [`DynSolType::Bytes`] or [`DynSolType::String`] that span multiple slots,
    /// use [`SlotInfo::decode_bytes_or_string`].
    pub fn decode(&self, value: B256) -> Option<DynSolValue> {
        // Storage values are always 32 bytes, stored as a single word
        let mut actual_type = &self.slot_type.dyn_sol_type;
        // Unwrap nested arrays to get to the base element type.
        while let DynSolType::FixedArray(elem_type, _) = actual_type {
            actual_type = elem_type.as_ref();
        }

        // Special handling for bytes and string types
        match actual_type {
            DynSolType::Bytes | DynSolType::String => {
                // Decode bytes/string from storage
                // The last byte contains the length * 2 for short strings/bytes
                // or length * 2 + 1 for long strings/bytes
                let length_byte = value.0[31];

                if length_byte & 1 == 0 {
                    // Short string/bytes (less than 32 bytes)
                    let length = (length_byte >> 1) as usize;
                    // Extract data
                    let data = if length == 0 { Vec::new() } else { value.0[0..length].to_vec() };

                    // Create the appropriate value based on type
                    if matches!(actual_type, DynSolType::String) {
                        let str_val = if data.is_empty() {
                            String::new()
                        } else {
                            String::from_utf8(data).unwrap_or_default()
                        };
                        Some(DynSolValue::String(str_val))
                    } else {
                        Some(DynSolValue::Bytes(data))
                    }
                } else {
                    // Long string/bytes (32 bytes or more)
                    // The actual data is stored at keccak256(slot)
                    // Return None for long values - they need decode_bytes_or_string()
                    None
                }
            }
            _ => {
                // Decode based on the actual type
                actual_type.abi_decode(&value.0).ok()
            }
        }
    }

    /// Slot is of type [`DynSolType::Bytes`] or [`DynSolType::String`]
    pub fn is_bytes_or_string(&self) -> bool {
        matches!(self.slot_type.dyn_sol_type, DynSolType::Bytes | DynSolType::String)
    }

    /// Decodes a [`DynSolType::Bytes`] or [`DynSolType::String`] value
    /// that spans across multiple slots.
    pub fn decode_bytes_or_string(
        &mut self,
        base_slot: &B256,
        storage_values: &B256Map<B256>,
    ) -> Option<DynSolValue> {
        // Only process bytes/string types
        if !self.is_bytes_or_string() {
            return None;
        }

        // Try to handle as long bytes/string
        self.aggregate_bytes_or_strings(base_slot, storage_values).map(|data| {
            match self.slot_type.dyn_sol_type {
                DynSolType::String => {
                    DynSolValue::String(String::from_utf8(data).unwrap_or_default())
                }
                DynSolType::Bytes => DynSolValue::Bytes(data),
                _ => unreachable!(),
            }
        })
    }

    /// Decodes both previous and new [`DynSolType::Bytes`] or [`DynSolType::String`] values
    /// that span across multiple slots using state diff data.
    ///
    /// Accepts a mapping of storage_slot to (previous_value, new_value).
    pub fn decode_bytes_or_string_values(
        &mut self,
        base_slot: &B256,
        storage_accesses: &BTreeMap<B256, (B256, B256)>,
    ) {
        // Only process bytes/string types
        if !self.is_bytes_or_string() {
            return;
        }

        // Get both previous and new values from the storage accesses
        if let Some((prev_base_value, new_base_value)) = storage_accesses.get(base_slot) {
            // Reusable closure to decode bytes/string based on length encoding
            let mut decode_value = |base_value: B256, is_new: bool| {
                let length_byte = base_value.0[31];
                if length_byte & 1 == 1 {
                    // Long bytes/string - aggregate from multiple slots
                    let value_map = storage_accesses
                        .iter()
                        .map(|(slot, (prev, new))| (*slot, if is_new { *new } else { *prev }))
                        .collect::<B256Map<_>>();
                    self.decode_bytes_or_string(base_slot, &value_map)
                } else {
                    // Short bytes/string - decode directly from base slot
                    self.decode(base_value)
                }
            };

            // Decode previous value
            let prev_decoded = decode_value(*prev_base_value, false);

            // Decode new value
            let new_decoded = decode_value(*new_base_value, true);

            // Set decoded values if both were successfully decoded
            if let (Some(prev), Some(new)) = (prev_decoded, new_decoded) {
                self.decoded = Some(DecodedSlotValues { previous_value: prev, new_value: new });
            }
        }
    }

    /// Aggregates a [`DynSolType::Bytes`] or [`DynSolType::String`] value that spans across
    /// multiple slots by looking up the length in the base_slot.
    ///
    /// Returns the aggregated raw bytes.
    fn aggregate_bytes_or_strings(
        &mut self,
        base_slot: &B256,
        storage_values: &B256Map<B256>,
    ) -> Option<Vec<u8>> {
        if !self.is_bytes_or_string() {
            return None;
        }

        // Check if it's a long bytes/string by looking at the base value
        if let Some(base_value) = storage_values.get(base_slot) {
            let length_byte = base_value.0[31];

            // Check if value is long
            if length_byte & 1 == 1 {
                // Long bytes/string - populate members
                let length: U256 = U256::from_be_bytes(base_value.0) >> 1;
                let num_slots = length.to::<usize>().div_ceil(32).min(256);
                let data_start = U256::from_be_bytes(keccak256(base_slot.0).0);

                let mut members = Vec::new();
                let mut full_data = Vec::with_capacity(length.to::<usize>());

                for i in 0..num_slots {
                    let data_slot = B256::from(data_start + U256::from(i));
                    let data_slot_u256 = data_start + U256::from(i);

                    // Create member info for this data slot with indexed label
                    let member_info = Self {
                        label: format!("{}[{}]", self.label, i),
                        slot_type: StorageTypeInfo {
                            label: self.slot_type.label.clone(),
                            dyn_sol_type: DynSolType::FixedBytes(32),
                        },
                        offset: 0,
                        slot: data_slot_u256.to_string(),
                        members: None,
                        decoded: None,
                        keys: None,
                    };

                    if let Some(value) = storage_values.get(&data_slot) {
                        // Collect data
                        let bytes_to_take =
                            std::cmp::min(32, length.to::<usize>() - full_data.len());
                        full_data.extend_from_slice(&value.0[..bytes_to_take]);
                    }

                    members.push(member_info);
                }

                // Set the members field
                if !members.is_empty() {
                    self.members = Some(members);
                }

                return Some(full_data);
            }
        }

        None
    }

    /// Decodes storage values (previous and new) and populates the decoded field.
    /// For structs with members, it decodes each member individually.
    pub fn decode_values(&mut self, previous_value: B256, new_value: B256) {
        // If this is a struct with members, decode each member individually
        if let Some(members) = &mut self.members {
            for member in members.iter_mut() {
                let offset = member.offset as usize;
                let size = match &member.slot_type.dyn_sol_type {
                    DynSolType::Uint(bits) | DynSolType::Int(bits) => bits / 8,
                    DynSolType::Address => 20,
                    DynSolType::Bool => 1,
                    DynSolType::FixedBytes(size) => *size,
                    _ => 32, // Default to full word
                };

                // Extract and decode member values
                let mut prev_bytes = [0u8; 32];
                let mut new_bytes = [0u8; 32];

                if offset + size <= 32 {
                    // In Solidity storage, values are right-aligned
                    // For offset 0, we want the rightmost bytes
                    // For offset 16 (for a uint128), we want bytes 0-16
                    // For packed storage: offset 0 is at the rightmost position
                    // offset 0, size 16 -> read bytes 16-32 (rightmost)
                    // offset 16, size 16 -> read bytes 0-16 (leftmost)
                    let byte_start = 32 - offset - size;
                    prev_bytes[32 - size..]
                        .copy_from_slice(&previous_value.0[byte_start..byte_start + size]);
                    new_bytes[32 - size..]
                        .copy_from_slice(&new_value.0[byte_start..byte_start + size]);
                }

                // Decode the member values
                if let (Ok(prev_val), Ok(new_val)) = (
                    member.slot_type.dyn_sol_type.abi_decode(&prev_bytes),
                    member.slot_type.dyn_sol_type.abi_decode(&new_bytes),
                ) {
                    member.decoded =
                        Some(DecodedSlotValues { previous_value: prev_val, new_value: new_val });
                }
            }
            // For structs with members, we don't need a top-level decoded value
        } else {
            // For non-struct types, decode directly
            // Note: decode() returns None for long bytes/strings, which will be handled by
            // decode_bytes_or_string()
            if let (Some(prev), Some(new)) = (self.decode(previous_value), self.decode(new_value)) {
                self.decoded = Some(DecodedSlotValues { previous_value: prev, new_value: new });
            }
        }
    }
}

/// Custom serializer for StorageTypeInfo that only outputs the label
fn serialize_slot_type<S>(info: &StorageTypeInfo, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    serializer.serialize_str(&info.label)
}

/// Custom serializer for mapping keys
fn serialize_mapping_keys<S>(keys: &Option<Vec<String>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    use serde::ser::SerializeMap;

    if let Some(keys) = keys {
        let len = if keys.is_empty() { 0 } else { 1 };
        let mut map = serializer.serialize_map(Some(len))?;
        if keys.len() == 1 {
            map.serialize_entry("key", &keys[0])?;
        } else if keys.len() > 1 {
            map.serialize_entry("keys", keys)?;
        }
        map.end()
    } else {
        serializer.serialize_none()
    }
}

/// Decoded storage slot values
#[derive(Debug)]
pub struct DecodedSlotValues {
    /// Initial decoded storage value
    pub previous_value: DynSolValue,
    /// Current decoded storage value
    pub new_value: DynSolValue,
}

impl Serialize for DecodedSlotValues {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("DecodedSlotValues", 2)?;
        state.serialize_field("previousValue", &format_token_raw(&self.previous_value))?;
        state.serialize_field("newValue", &format_token_raw(&self.new_value))?;
        state.end()
    }
}

/// Storage slot identifier that uses Solidity [`StorageLayout`] to identify storage slots.
pub struct SlotIdentifier {
    storage_layout: Arc<StorageLayout>,
}

impl SlotIdentifier {
    /// Creates a new SlotIdentifier with the given storage layout.
    pub fn new(storage_layout: Arc<StorageLayout>) -> Self {
        Self { storage_layout }
    }

    /// Identifies a storage slots type using the [`StorageLayout`].
    ///
    /// It can also identify whether a slot belongs to a mapping if provided with [`MappingSlots`].
    pub fn identify(&self, slot: &B256, mapping_slots: Option<&MappingSlots>) -> Option<SlotInfo> {
        trace!(?slot, "identifying slot");
        let slot_u256 = U256::from_be_bytes(slot.0);
        let slot_str = slot_u256.to_string();

        for storage in &self.storage_layout.storage {
            let storage_type = self.storage_layout.types.get(&storage.storage_type)?;
            let dyn_type = DynSolType::parse(&storage_type.label).ok();

            // Check if we're able to match on a slot from the layout i.e any of the base slots.
            // This will always be the case for primitive types that fit in a single slot.
            if storage.slot == slot_str
                && let Some(parsed_type) = dyn_type
            {
                // Successfully parsed - handle arrays or simple types
                let label = if let DynSolType::FixedArray(_, _) = &parsed_type {
                    format!("{}{}", storage.label, get_array_base_indices(&parsed_type))
                } else {
                    storage.label.clone()
                };

                return Some(SlotInfo {
                    label,
                    slot_type: StorageTypeInfo {
                        label: storage_type.label.clone(),
                        dyn_sol_type: parsed_type,
                    },
                    offset: storage.offset,
                    slot: storage.slot.clone(),
                    members: None,
                    decoded: None,
                    keys: None,
                });
            }

            // Encoding types: <https://docs.soliditylang.org/en/latest/internals/layout_in_storage.html#json-output>
            if storage_type.encoding == ENCODING_INPLACE {
                // Can be of type FixedArrays or Structs
                // Handles the case where the accessed `slot` is maybe different from the base slot.
                let array_start_slot = U256::from_str(&storage.slot).ok()?;

                if let Some(parsed_type) = dyn_type
                    && let DynSolType::FixedArray(_, _) = parsed_type
                    && let Some(slot_info) = self.handle_array_slot(
                        storage,
                        storage_type,
                        slot_u256,
                        array_start_slot,
                        &slot_str,
                    )
                {
                    return Some(slot_info);
                }

                // If type parsing fails and the label is a struct
                if is_struct(&storage_type.label) {
                    let struct_start_slot = U256::from_str(&storage.slot).ok()?;
                    if let Some(slot_info) = self.handle_struct(
                        &storage.label,
                        storage_type,
                        slot_u256,
                        struct_start_slot,
                        storage.offset,
                        &slot_str,
                        0,
                    ) {
                        return Some(slot_info);
                    }
                }
            } else if storage_type.encoding == ENCODING_MAPPING
                && let Some(mapping_slots) = mapping_slots
                && let Some(info) =
                    self.handle_mapping(storage, storage_type, slot, &slot_str, mapping_slots)
            {
                return Some(info);
            }
        }

        None
    }

    /// Identifies a bytes or string storage slot by checking all bytes/string variables
    /// in the storage layout and using their base slot values from the provided storage changes.
    ///
    /// # Arguments
    /// * `slot` - The slot being identified
    /// * `storage_values` - Map of storage slots to their current values
    pub fn identify_bytes_or_string(
        &self,
        slot: &B256,
        storage_values: &B256Map<B256>,
    ) -> Option<SlotInfo> {
        let slot_u256 = U256::from_be_bytes(slot.0);
        let slot_str = slot_u256.to_string();

        // Search through all bytes/string variables in the storage layout
        for storage in &self.storage_layout.storage {
            if let Some(storage_type) = self.storage_layout.types.get(&storage.storage_type)
                && storage_type.encoding == ENCODING_BYTES
            {
                let Some(base_slot) = U256::from_str(&storage.slot).map(B256::from).ok() else {
                    continue;
                };
                // Get the base slot value from storage_values
                if let Some(base_value) = storage_values.get(&base_slot)
                    && let Some(info) = self.handle_bytes_string(
                        storage,
                        storage_type,
                        slot_u256,
                        &slot_str,
                        base_value,
                    )
                {
                    return Some(info);
                }
            }
        }

        None
    }

    /// Handles identification of array slots.
    ///
    /// # Arguments
    /// * `storage` - The storage metadata from the layout
    /// * `storage_type` - Type information for the storage slot
    /// * `slot` - The target slot being identified
    /// * `array_start_slot` - The starting slot of the array in storage i.e base_slot
    /// * `slot_str` - String representation of the slot for output
    fn handle_array_slot(
        &self,
        storage: &Storage,
        storage_type: &StorageType,
        slot: U256,
        array_start_slot: U256,
        slot_str: &str,
    ) -> Option<SlotInfo> {
        // Check if slot is within array bounds
        let total_bytes = storage_type.number_of_bytes.parse::<u64>().ok()?;
        let total_slots = total_bytes.div_ceil(32);

        if slot >= array_start_slot && slot < array_start_slot + U256::from(total_slots) {
            let parsed_type = DynSolType::parse(&storage_type.label).ok()?;
            let index = (slot - array_start_slot).to::<u64>();
            // Format the array element label based on array dimensions
            let label = match &parsed_type {
                DynSolType::FixedArray(inner, _) => {
                    if let DynSolType::FixedArray(_, inner_size) = inner.as_ref() {
                        // 2D array: calculate row and column
                        let row = index / (*inner_size as u64);
                        let col = index % (*inner_size as u64);
                        format!("{}[{row}][{col}]", storage.label)
                    } else {
                        // 1D array
                        format!("{}[{index}]", storage.label)
                    }
                }
                _ => storage.label.clone(),
            };

            return Some(SlotInfo {
                label,
                slot_type: StorageTypeInfo {
                    label: storage_type.label.clone(),
                    dyn_sol_type: parsed_type,
                },
                offset: 0,
                slot: slot_str.to_string(),
                members: None,
                decoded: None,
                keys: None,
            });
        }

        None
    }

    /// Handles identification of struct slots.
    ///
    /// Recursively resolves struct members to find the exact member corresponding
    /// to the target slot. Handles both single-slot (packed) and multi-slot structs.
    ///
    /// # Arguments
    /// * `base_label` - The label/name for this struct or member
    /// * `storage_type` - Type information for the storage
    /// * `target_slot` - The target slot being identified
    /// * `struct_start_slot` - The starting slot of this struct
    /// * `offset` - Offset within the slot (for packed storage)
    /// * `slot_str` - String representation of the slot for output
    /// * `depth` - Current recursion depth
    #[allow(clippy::too_many_arguments)]
    fn handle_struct(
        &self,
        base_label: &str,
        storage_type: &StorageType,
        target_slot: U256,
        struct_start_slot: U256,
        offset: i64,
        slot_str: &str,
        depth: usize,
    ) -> Option<SlotInfo> {
        // Limit recursion depth to prevent stack overflow
        const MAX_DEPTH: usize = 10;
        if depth > MAX_DEPTH {
            return None;
        }

        let members = storage_type
            .other
            .get("members")
            .and_then(|v| serde_json::from_value::<Vec<Storage>>(v.clone()).ok())?;

        // If this is the exact slot we're looking for (struct's base slot)
        if struct_start_slot == target_slot
        // Find the member at slot offset 0 (the member that starts at this slot)
            && let Some(first_member) = members.iter().find(|m| m.slot == "0")
        {
            let member_type_info = self.storage_layout.types.get(&first_member.storage_type)?;

            // Check if we have a single-slot struct (all members have slot "0")
            let is_single_slot = members.iter().all(|m| m.slot == "0");

            if is_single_slot {
                // Build member info for single-slot struct
                let mut member_infos = Vec::new();
                for member in &members {
                    if let Some(member_type_info) =
                        self.storage_layout.types.get(&member.storage_type)
                        && let Some(member_type) = DynSolType::parse(&member_type_info.label).ok()
                    {
                        member_infos.push(SlotInfo {
                            label: member.label.clone(),
                            slot_type: StorageTypeInfo {
                                label: member_type_info.label.clone(),
                                dyn_sol_type: member_type,
                            },
                            offset: member.offset,
                            slot: slot_str.to_string(),
                            members: None,
                            decoded: None,
                            keys: None,
                        });
                    }
                }

                // Build the CustomStruct type
                let struct_name =
                    storage_type.label.strip_prefix("struct ").unwrap_or(&storage_type.label);
                let prop_names: Vec<String> = members.iter().map(|m| m.label.clone()).collect();
                let member_types: Vec<DynSolType> =
                    member_infos.iter().map(|info| info.slot_type.dyn_sol_type.clone()).collect();

                let parsed_type = DynSolType::CustomStruct {
                    name: struct_name.to_string(),
                    prop_names,
                    tuple: member_types,
                };

                return Some(SlotInfo {
                    label: base_label.to_string(),
                    slot_type: StorageTypeInfo {
                        label: storage_type.label.clone(),
                        dyn_sol_type: parsed_type,
                    },
                    offset,
                    slot: slot_str.to_string(),
                    decoded: None,
                    members: if member_infos.is_empty() { None } else { Some(member_infos) },
                    keys: None,
                });
            } else {
                // Multi-slot struct - return the first member.
                let member_label = format!("{}.{}", base_label, first_member.label);

                // If the first member is itself a struct, recurse
                if is_struct(&member_type_info.label) {
                    return self.handle_struct(
                        &member_label,
                        member_type_info,
                        target_slot,
                        struct_start_slot,
                        first_member.offset,
                        slot_str,
                        depth + 1,
                    );
                }

                // Return the first member as a primitive
                return Some(SlotInfo {
                    label: member_label,
                    slot_type: StorageTypeInfo {
                        label: member_type_info.label.clone(),
                        dyn_sol_type: DynSolType::parse(&member_type_info.label).ok()?,
                    },
                    offset: first_member.offset,
                    slot: slot_str.to_string(),
                    decoded: None,
                    members: None,
                    keys: None,
                });
            }
        }

        // Not the base slot - search through members
        for member in &members {
            let member_slot_offset = U256::from_str(&member.slot).ok()?;
            let member_slot = struct_start_slot + member_slot_offset;
            let member_type_info = self.storage_layout.types.get(&member.storage_type)?;
            let member_label = format!("{}.{}", base_label, member.label);

            // If this member is a struct, recurse into it
            if is_struct(&member_type_info.label) {
                let slot_info = self.handle_struct(
                    &member_label,
                    member_type_info,
                    target_slot,
                    member_slot,
                    member.offset,
                    slot_str,
                    depth + 1,
                );

                if member_slot == target_slot || slot_info.is_some() {
                    return slot_info;
                }
            }

            if member_slot == target_slot {
                // Found the exact member slot

                // Regular member
                let member_type = DynSolType::parse(&member_type_info.label).ok()?;
                return Some(SlotInfo {
                    label: member_label,
                    slot_type: StorageTypeInfo {
                        label: member_type_info.label.clone(),
                        dyn_sol_type: member_type,
                    },
                    offset: member.offset,
                    slot: slot_str.to_string(),
                    members: None,
                    decoded: None,
                    keys: None,
                });
            }
        }

        None
    }

    /// Handles identification of mapping slots.
    ///
    /// Identifies mapping entries by walking up the parent chain to find the base slot,
    /// then decodes the keys and builds the appropriate label.
    ///
    /// # Arguments
    /// * `storage` - The storage metadata from the layout
    /// * `storage_type` - Type information for the storage
    /// * `slot` - The accessed slot being identified
    /// * `slot_str` - String representation of the slot for output
    /// * `mapping_slots` - Tracked mapping slot accesses for key resolution
    fn handle_mapping(
        &self,
        storage: &Storage,
        storage_type: &StorageType,
        slot: &B256,
        slot_str: &str,
        mapping_slots: &MappingSlots,
    ) -> Option<SlotInfo> {
        trace!(
            "handle_mapping: storage.slot={}, slot={:?}, has_keys={}, has_parents={}",
            storage.slot,
            slot,
            mapping_slots.keys.contains_key(slot),
            mapping_slots.parent_slots.contains_key(slot)
        );

        // Verify it's actually a mapping type
        if storage_type.encoding != ENCODING_MAPPING {
            return None;
        }

        // Check if this slot is a known mapping entry
        if !mapping_slots.keys.contains_key(slot) {
            return None;
        }

        // Convert storage.slot to B256 for comparison
        let storage_slot_b256 = B256::from(U256::from_str(&storage.slot).ok()?);

        // Walk up the parent chain to collect keys and validate the base slot
        let mut current_slot = *slot;
        let mut keys_to_decode = Vec::new();
        let mut found_base = false;

        while let Some((key, parent)) =
            mapping_slots.keys.get(&current_slot).zip(mapping_slots.parent_slots.get(&current_slot))
        {
            keys_to_decode.push(*key);

            // Check if the parent is our base storage slot
            if *parent == storage_slot_b256 {
                found_base = true;
                break;
            }

            // Move up to the parent for the next iteration
            current_slot = *parent;
        }

        if !found_base {
            trace!("Mapping slot {} does not match any parent in chain", storage.slot);
            return None;
        }

        // Resolve the mapping type to get all key types and the final value type
        let (key_types, value_type_label, full_type_label) =
            self.resolve_mapping_type(&storage.storage_type)?;

        // Reverse keys to process from outermost to innermost
        keys_to_decode.reverse();

        // Build the label with decoded keys and collect decoded key values
        let mut label = storage.label.clone();
        let mut decoded_keys = Vec::new();

        // Decode each key using the corresponding type
        for (i, key) in keys_to_decode.iter().enumerate() {
            if let Some(key_type_label) = key_types.get(i)
                && let Ok(sol_type) = DynSolType::parse(key_type_label)
                && let Ok(decoded) = sol_type.abi_decode(&key.0)
            {
                let decoded_key_str = format_token_raw(&decoded);
                decoded_keys.push(decoded_key_str.clone());
                label = format!("{label}[{decoded_key_str}]");
            } else {
                let hex_key = hex::encode_prefixed(key.0);
                decoded_keys.push(hex_key.clone());
                label = format!("{label}[{hex_key}]");
            }
        }

        // Parse the final value type for decoding
        let dyn_sol_type = DynSolType::parse(&value_type_label).unwrap_or(DynSolType::Bytes);

        Some(SlotInfo {
            label,
            slot_type: StorageTypeInfo { label: full_type_label, dyn_sol_type },
            offset: storage.offset,
            slot: slot_str.to_string(),
            members: None,
            decoded: None,
            keys: Some(decoded_keys),
        })
    }

    /// Handles identification of bytes/string storage slots.
    ///
    /// Bytes and strings in Solidity use a special storage layout:
    /// - Short values (<32 bytes): stored in the same slot with length * 2
    /// - Long values (>=32 bytes): length * 2 + 1 in main slot, data at keccak256(slot)
    ///
    /// This function checks if the given slot is:
    /// 1. A main slot for a bytes/string variable
    /// 2. A data slot for any long bytes/string variable in the storage layout
    ///
    /// # Arguments
    /// * `slot` - The accessed slot being identified
    /// * `slot_str` - String representation of the slot for output
    /// * `base_slot_value` - The value at the base slot (used to determine length for long
    ///   bytes/strings)
    fn handle_bytes_string(
        &self,
        storage: &Storage,
        storage_type: &StorageType,
        slot: U256,
        slot_str: &str,
        base_slot_value: &B256,
    ) -> Option<SlotInfo> {
        // Only handle bytes/string encoded variables for this specific storage entry
        if storage_type.encoding != ENCODING_BYTES {
            return None;
        }

        // Check if this is the main slot for this variable
        let base_slot = U256::from_str(&storage.slot).ok()?;
        if slot == base_slot {
            // Parse the type to get the correct DynSolType
            let dyn_type = if storage_type.label == "string" {
                DynSolType::String
            } else if storage_type.label == "bytes" {
                DynSolType::Bytes
            } else {
                return None;
            };

            return Some(SlotInfo {
                label: storage.label.clone(),
                slot_type: StorageTypeInfo {
                    label: storage_type.label.clone(),
                    dyn_sol_type: dyn_type,
                },
                offset: storage.offset,
                slot: slot_str.to_string(),
                members: None,
                decoded: None,
                keys: None,
            });
        }

        // Check if it could be a data slot for this long bytes/string
        // Calculate where data slots would start for this variable
        let data_start =
            U256::from_be_bytes(alloy_primitives::keccak256(base_slot.to_be_bytes::<32>()).0);

        // Get the length from the base slot value to calculate exact number of slots
        // For long bytes/strings, the length is stored as (length * 2 + 1) in the base slot
        let length_byte = base_slot_value.0[31];
        if length_byte & 1 == 1 {
            // It's a long bytes/string
            let length = U256::from_be_bytes(base_slot_value.0) >> 1;
            // Calculate number of slots needed (round up)
            let num_slots = (length + U256::from(31)) / U256::from(32);

            // Check if our slot is within the data region
            if slot >= data_start && slot < data_start + num_slots {
                let slot_index = (slot - data_start).to::<usize>();

                return Some(SlotInfo {
                    label: format!("{}[{}]", storage.label, slot_index),
                    slot_type: StorageTypeInfo {
                        label: storage_type.label.clone(),
                        // Type is assigned as FixedBytes(32) for data slots
                        dyn_sol_type: DynSolType::FixedBytes(32),
                    },
                    offset: 0,
                    slot: slot_str.to_string(),
                    members: None,
                    decoded: None,
                    keys: None,
                });
            }
        }

        None
    }

    fn resolve_mapping_type(&self, type_ref: &str) -> Option<(Vec<String>, String, String)> {
        let storage_type = self.storage_layout.types.get(type_ref)?;

        if storage_type.encoding != ENCODING_MAPPING {
            // Not a mapping, return the type as-is
            return Some((vec![], storage_type.label.clone(), storage_type.label.clone()));
        }

        // Get key and value type references
        let key_type_ref = storage_type.key.as_ref()?;
        let value_type_ref = storage_type.value.as_ref()?;

        // Resolve the key type
        let key_type = self.storage_layout.types.get(key_type_ref)?;
        let mut key_types = vec![key_type.label.clone()];

        // Check if the value is another mapping (nested case)
        if let Some(value_storage_type) = self.storage_layout.types.get(value_type_ref) {
            if value_storage_type.encoding == ENCODING_MAPPING {
                // Recursively resolve the nested mapping
                let (nested_keys, final_value, _) = self.resolve_mapping_type(value_type_ref)?;
                key_types.extend(nested_keys);
                return Some((key_types, final_value, storage_type.label.clone()));
            } else {
                // Value is not a mapping, we're done
                return Some((
                    key_types,
                    value_storage_type.label.clone(),
                    storage_type.label.clone(),
                ));
            }
        }

        None
    }
}

/// Returns the base indices for array types, e.g. "\[0\]\[0\]" for 2D arrays.
fn get_array_base_indices(dyn_type: &DynSolType) -> String {
    match dyn_type {
        DynSolType::FixedArray(inner, _) => {
            if let DynSolType::FixedArray(_, _) = inner.as_ref() {
                // Nested array (2D or higher)
                format!("[0]{}", get_array_base_indices(inner))
            } else {
                // Simple 1D array
                "[0]".to_string()
            }
        }
        _ => String::new(),
    }
}

/// Checks if a given type label represents a struct type.
pub fn is_struct(s: &str) -> bool {
    s.starts_with("struct ")
}
