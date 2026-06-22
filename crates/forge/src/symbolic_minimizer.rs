use crate::result::SymbolicCounterexampleCall;
use alloy_dyn_abi::{DynSolValue, JsonAbiExt};
use alloy_json_abi::Function;
use alloy_primitives::{Address, B256, Bytes, Function as SolFunction, I256, U256};
use itertools::Itertools;

const MAX_MINIMIZATION_ATTEMPTS: usize = 5000;
const MAX_SUBSET_CANDIDATES_PER_PASS: usize = 256;

/// Result of deterministic single-call counterexample minimization.
#[derive(Clone, Debug)]
pub(crate) struct MinimizedSingleCall {
    pub original_call: SymbolicCounterexampleCall,
    pub minimized_call: SymbolicCounterexampleCall,
    pub attempts: usize,
    pub accepted: usize,
}

impl MinimizedSingleCall {
    pub(crate) fn changed(&self) -> bool {
        self.original_call.calldata != self.minimized_call.calldata
            || self.original_call.sender != self.minimized_call.sender
            || self.original_call.target != self.minimized_call.target
            || self.original_call.value != self.minimized_call.value
            || self.original_call.warp != self.minimized_call.warp
            || self.original_call.roll != self.minimized_call.roll
    }
}

/// Minimizes a stateless symbolic counterexample with ABI-valid candidates only.
///
/// `still_fails` must concretely replay the candidate and return `true` only when it preserves the
/// already-confirmed failure.
pub(crate) fn minimize_single_call_counterexample(
    function: &Function,
    call: &SymbolicCounterexampleCall,
    configured_senders: &[Address],
    mut still_fails: impl FnMut(&SymbolicCounterexampleCall) -> bool,
) -> Option<MinimizedSingleCall> {
    if call.calldata.get(..4).is_none_or(|selector| selector != function.selector()) {
        return None;
    }

    let original_args = function.abi_decode_input(&call.calldata[4..]).ok()?;
    let mut current_args = original_args;
    let mut current_call = call_with_args(function, call, &current_args)?;
    let mut attempts = 0usize;
    let mut accepted = 0usize;

    let mut try_args = |candidate_args: &[DynSolValue]| {
        let Some(candidate_call) = call_with_args(function, call, candidate_args) else {
            return false;
        };
        if candidate_call.calldata == current_call.calldata {
            return false;
        }
        if attempts >= MAX_MINIMIZATION_ATTEMPTS {
            return false;
        }

        attempts += 1;
        if still_fails(&candidate_call) {
            current_call = candidate_call;
            accepted += 1;
            true
        } else {
            false
        }
    };
    minimize_values_batch(&mut current_args, &mut try_args);
    minimize_value_subsets(&mut current_args, &mut try_args);
    minimize_values(&mut current_args, &mut try_args);
    minimize_call_fields(
        &mut current_call,
        configured_senders,
        &mut attempts,
        &mut accepted,
        &mut still_fails,
    );

    current_call = with_formatted_args(current_call, &current_args);

    Some(MinimizedSingleCall {
        original_call: call.clone(),
        minimized_call: current_call,
        attempts,
        accepted,
    })
}

fn call_with_args(
    function: &Function,
    template: &SymbolicCounterexampleCall,
    args: &[DynSolValue],
) -> Option<SymbolicCounterexampleCall> {
    let calldata = Bytes::from(function.abi_encode_input(args).ok()?);
    Some(SymbolicCounterexampleCall { calldata, args: None, raw_args: None, ..template.clone() })
}

fn with_formatted_args(
    mut call: SymbolicCounterexampleCall,
    args: &[DynSolValue],
) -> SymbolicCounterexampleCall {
    call.args = Some(foundry_common::fmt::format_tokens(args).format(", ").to_string());
    call.raw_args = Some(foundry_common::fmt::format_tokens_raw(args).format(", ").to_string());
    call
}

fn minimize_values_batch(
    values: &mut Vec<DynSolValue>,
    try_values: &mut dyn FnMut(&[DynSolValue]) -> bool,
) -> bool {
    let candidate_values = values.iter().cloned().map(minimally_simple_value).collect::<Vec<_>>();
    if candidate_values == *values {
        return false;
    }
    if try_values(&candidate_values) {
        *values = candidate_values;
        true
    } else {
        false
    }
}

fn minimize_value_subsets(
    values: &mut Vec<DynSolValue>,
    try_values: &mut dyn FnMut(&[DynSolValue]) -> bool,
) -> bool {
    let mut changed = false;
    loop {
        let simple_values = values.iter().cloned().map(minimally_simple_value).collect::<Vec<_>>();
        let shrinkable_idxs = values
            .iter()
            .zip(&simple_values)
            .enumerate()
            .filter_map(|(idx, (current, simple))| (current != simple).then_some(idx))
            .collect::<Vec<_>>();
        if shrinkable_idxs.len() < 2 {
            break;
        }

        let mut pass_changed = false;
        for subset_size in subset_sizes(shrinkable_idxs.len()) {
            let mut subset = Vec::with_capacity(subset_size);
            if try_value_subset(
                values,
                &simple_values,
                &shrinkable_idxs,
                subset_size,
                0,
                &mut subset,
                try_values,
            ) {
                pass_changed = true;
                break;
            }
        }

        if !pass_changed {
            break;
        }
        changed = true;
    }
    changed
}

fn try_value_subset(
    values: &mut Vec<DynSolValue>,
    simple_values: &[DynSolValue],
    shrinkable_idxs: &[usize],
    subset_size: usize,
    start: usize,
    subset: &mut Vec<usize>,
    try_values: &mut dyn FnMut(&[DynSolValue]) -> bool,
) -> bool {
    if subset.len() == subset_size {
        let mut candidate_values = values.clone();
        for idx in subset.iter().copied() {
            candidate_values[idx] = simple_values[idx].clone();
        }
        if try_values(&candidate_values) {
            *values = candidate_values;
            return true;
        }
        return false;
    }

    let remaining = subset_size - subset.len();
    for choice_idx in start..=shrinkable_idxs.len() - remaining {
        subset.push(shrinkable_idxs[choice_idx]);
        if try_value_subset(
            values,
            simple_values,
            shrinkable_idxs,
            subset_size,
            choice_idx + 1,
            subset,
            try_values,
        ) {
            return true;
        }
        subset.pop();
    }
    false
}

fn minimally_simple_value(mut value: DynSolValue) -> DynSolValue {
    minimize_value(&mut value, &mut |_| true);
    value
}

fn minimize_call_fields(
    call: &mut SymbolicCounterexampleCall,
    configured_senders: &[Address],
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
) {
    loop {
        let changed =
            minimize_call_sender(call, configured_senders, attempts, accepted, still_fails)
                || minimize_call_u256_field(
                    call,
                    attempts,
                    accepted,
                    still_fails,
                    |call| call.value,
                    |call, value| call.value = value,
                )
                || minimize_call_delay(call, attempts, accepted, still_fails);
        if !changed {
            break;
        }
    }
}

fn minimize_call_sender(
    call: &mut SymbolicCounterexampleCall,
    configured_senders: &[Address],
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
) -> bool {
    let mut ordered_senders = configured_senders.to_vec();
    ordered_senders.sort_unstable();
    ordered_senders.dedup();
    let Some(current_idx) = ordered_senders.iter().position(|sender| *sender == call.sender) else {
        return false;
    };

    for candidate in ordered_senders.into_iter().take(current_idx) {
        let mut candidate_call = call.clone();
        candidate_call.sender = candidate;
        if accept_call_candidate(call, candidate_call, attempts, accepted, still_fails) {
            return true;
        }
    }
    false
}

fn minimize_call_delay(
    call: &mut SymbolicCounterexampleCall,
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
) -> bool {
    match (call.warp, call.roll) {
        (Some(warp), Some(roll)) => {
            minimize_call_delay_pair(call, warp, roll, attempts, accepted, still_fails)
        }
        _ => {
            minimize_call_u256_field(
                call,
                attempts,
                accepted,
                still_fails,
                |call| call.warp,
                |call, value| call.warp = value,
            ) || minimize_call_u256_field(
                call,
                attempts,
                accepted,
                still_fails,
                |call| call.roll,
                |call, value| call.roll = value,
            )
        }
    }
}

fn minimize_call_delay_pair(
    call: &mut SymbolicCounterexampleCall,
    current_warp: U256,
    current_roll: U256,
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
) -> bool {
    if current_warp.is_zero() && current_roll.is_zero() {
        return false;
    }

    let mut try_delay = |call: &mut SymbolicCounterexampleCall, warp: U256, roll: U256| {
        let (warp, roll) = level_delay(warp, roll);
        let mut candidate_call = call.clone();
        candidate_call.warp = Some(warp);
        candidate_call.roll = Some(roll);
        accept_call_candidate(call, candidate_call, attempts, accepted, still_fails)
    };

    (!current_roll.is_zero()
        && minimize_u256_candidates(current_roll, |candidate| {
            try_delay(call, current_warp, candidate)
        }))
        || (!current_warp.is_zero()
            && minimize_u256_candidates(current_warp, |candidate| {
                try_delay(call, candidate, current_roll)
            }))
        || minimize_u256_pair_candidates(current_warp, current_roll, |warp, roll| {
            try_delay(call, warp, roll)
        })
}

fn level_delay(warp: U256, roll: U256) -> (U256, U256) {
    if warp.is_zero() || roll.is_zero() { (U256::ZERO, U256::ZERO) } else { (warp, roll) }
}

fn minimize_call_u256_field(
    call: &mut SymbolicCounterexampleCall,
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
    get: impl Fn(&SymbolicCounterexampleCall) -> Option<U256>,
    set: impl Fn(&mut SymbolicCounterexampleCall, Option<U256>),
) -> bool {
    let Some(current) = get(call) else {
        return false;
    };
    if current.is_zero() {
        return false;
    }

    minimize_u256_candidates(current, |candidate| {
        let mut candidate_call = call.clone();
        set(&mut candidate_call, Some(candidate));
        accept_call_candidate(call, candidate_call, attempts, accepted, still_fails)
    })
}

fn minimize_u256_candidates(current: U256, mut try_candidate: impl FnMut(U256) -> bool) -> bool {
    if !current.is_zero() && try_candidate(U256::ZERO) {
        return true;
    }
    for bit in (0..256).rev() {
        let mask: U256 = U256::from(1) << bit;
        if current & mask == U256::ZERO {
            continue;
        }
        if try_candidate(current & !mask) {
            return true;
        }
    }

    let mut accepted_value = current;
    let mut rejected_value = U256::ZERO;
    let mut changed = false;
    while accepted_value > rejected_value + U256::from(1) {
        let candidate = rejected_value + ((accepted_value - rejected_value) >> 1usize);
        if try_candidate(candidate) {
            accepted_value = candidate;
            changed = true;
        } else {
            rejected_value = candidate;
        }
    }
    changed
}

fn minimize_u256_pair_candidates(
    current_left: U256,
    current_right: U256,
    mut try_candidate: impl FnMut(U256, U256) -> bool,
) -> bool {
    if current_left.is_zero() || current_right.is_zero() {
        return false;
    }

    let mut accepted_left = current_left;
    let mut accepted_right = current_right;
    let mut rejected_left = U256::ZERO;
    let mut rejected_right = U256::ZERO;
    let mut changed = false;
    while accepted_left > rejected_left + U256::from(1)
        || accepted_right > rejected_right + U256::from(1)
    {
        let candidate_left = if accepted_left > rejected_left + U256::from(1) {
            rejected_left + ((accepted_left - rejected_left) >> 1usize)
        } else {
            accepted_left
        };
        let candidate_right = if accepted_right > rejected_right + U256::from(1) {
            rejected_right + ((accepted_right - rejected_right) >> 1usize)
        } else {
            accepted_right
        };

        if try_candidate(candidate_left, candidate_right) {
            accepted_left = candidate_left;
            accepted_right = candidate_right;
            changed = true;
        } else {
            rejected_left = candidate_left;
            rejected_right = candidate_right;
        }
    }
    changed
}

fn accept_call_candidate(
    call: &mut SymbolicCounterexampleCall,
    candidate: SymbolicCounterexampleCall,
    attempts: &mut usize,
    accepted: &mut usize,
    still_fails: &mut impl FnMut(&SymbolicCounterexampleCall) -> bool,
) -> bool {
    if same_call(call, &candidate) || *attempts >= MAX_MINIMIZATION_ATTEMPTS {
        return false;
    }

    *attempts += 1;
    if still_fails(&candidate) {
        *call = candidate;
        *accepted += 1;
        true
    } else {
        false
    }
}

fn same_call(left: &SymbolicCounterexampleCall, right: &SymbolicCounterexampleCall) -> bool {
    left.warp == right.warp
        && left.roll == right.roll
        && left.sender == right.sender
        && left.target == right.target
        && left.calldata == right.calldata
        && left.value == right.value
}

fn minimize_values(values: &mut [DynSolValue], try_values: &mut dyn FnMut(&[DynSolValue]) -> bool) {
    loop {
        let mut changed = false;
        for idx in 0..values.len() {
            let mut value = values[idx].clone();
            let value_changed = minimize_value(&mut value, &mut |candidate| {
                let mut candidate_values = values.to_vec();
                candidate_values[idx] = candidate.clone();
                try_values(&candidate_values)
            });
            if value_changed {
                values[idx] = value;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

fn minimize_value(
    value: &mut DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    let mut changed = false;
    loop {
        let pass_changed =
            minimize_scalar_value(value, try_value) || minimize_compound_value(value, try_value);
        if !pass_changed {
            break;
        }
        changed = true;
    }
    changed
}

fn minimize_scalar_value(
    value: &mut DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    match value.clone() {
        DynSolValue::Bool(true) => accept_candidate(value, DynSolValue::Bool(false), try_value),
        DynSolValue::Bool(false) => false,
        DynSolValue::Uint(current, bits) => minimize_uint(value, current, bits, try_value),
        DynSolValue::Int(current, bits) => minimize_int(value, current, bits, try_value),
        DynSolValue::Address(current) => minimize_address(value, current, try_value),
        DynSolValue::FixedBytes(current, size) => {
            minimize_fixed_bytes(value, current, size, try_value)
        }
        DynSolValue::Function(current) => {
            if current == SolFunction::ZERO {
                false
            } else {
                accept_candidate(value, DynSolValue::Function(SolFunction::ZERO), try_value)
            }
        }
        DynSolValue::Bytes(current) => minimize_bytes(value, current, try_value),
        DynSolValue::String(current) => minimize_string(value, current, try_value),
        DynSolValue::Array(_)
        | DynSolValue::FixedArray(_)
        | DynSolValue::Tuple(_)
        | DynSolValue::CustomStruct { .. } => false,
    }
}

fn minimize_compound_value(
    value: &mut DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    match value.clone() {
        DynSolValue::Array(mut elements) => {
            if minimize_array_len(value, &mut elements, try_value) {
                return true;
            }
            if let Some(candidate) = minimize_elements_batch(
                &mut elements,
                |items| DynSolValue::Array(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            if let Some(candidate) = minimize_element_subsets(
                &mut elements,
                |items| DynSolValue::Array(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            minimize_elements(&mut elements, |items| DynSolValue::Array(items.to_vec()), try_value)
                .map(|candidate| {
                    *value = candidate;
                    true
                })
                .unwrap_or(false)
        }
        DynSolValue::FixedArray(mut elements) => {
            if let Some(candidate) = minimize_elements_batch(
                &mut elements,
                |items| DynSolValue::FixedArray(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            if let Some(candidate) = minimize_element_subsets(
                &mut elements,
                |items| DynSolValue::FixedArray(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            minimize_elements(
                &mut elements,
                |items| DynSolValue::FixedArray(items.to_vec()),
                try_value,
            )
            .map(|candidate| {
                *value = candidate;
                true
            })
            .unwrap_or(false)
        }
        DynSolValue::Tuple(mut elements) => {
            if let Some(candidate) = minimize_elements_batch(
                &mut elements,
                |items| DynSolValue::Tuple(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            if let Some(candidate) = minimize_element_subsets(
                &mut elements,
                |items| DynSolValue::Tuple(items.to_vec()),
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            minimize_elements(&mut elements, |items| DynSolValue::Tuple(items.to_vec()), try_value)
                .map(|candidate| {
                    *value = candidate;
                    true
                })
                .unwrap_or(false)
        }
        DynSolValue::CustomStruct { name, prop_names, mut tuple } => {
            if let Some(candidate) = minimize_elements_batch(
                &mut tuple,
                |items| DynSolValue::CustomStruct {
                    name: name.clone(),
                    prop_names: prop_names.clone(),
                    tuple: items.to_vec(),
                },
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            if let Some(candidate) = minimize_element_subsets(
                &mut tuple,
                |items| DynSolValue::CustomStruct {
                    name: name.clone(),
                    prop_names: prop_names.clone(),
                    tuple: items.to_vec(),
                },
                try_value,
            ) {
                *value = candidate;
                return true;
            }
            minimize_elements(
                &mut tuple,
                |items| DynSolValue::CustomStruct {
                    name: name.clone(),
                    prop_names: prop_names.clone(),
                    tuple: items.to_vec(),
                },
                try_value,
            )
            .map(|candidate| {
                *value = candidate;
                true
            })
            .unwrap_or(false)
        }
        _ => false,
    }
}

fn minimize_uint(
    value: &mut DynSolValue,
    current: U256,
    bits: usize,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    if !current.is_zero() && accept_candidate(value, DynSolValue::Uint(U256::ZERO, bits), try_value)
    {
        return true;
    }

    let bit_limit = bits.min(256);
    for bit in (0..bit_limit).rev() {
        let mask = U256::from(1) << bit;
        if current & mask == U256::ZERO {
            continue;
        }
        let candidate = current & !mask;
        if accept_candidate(value, DynSolValue::Uint(candidate, bits), try_value) {
            return true;
        }
    }

    minimize_uint_by_search(value, current, bits, try_value)
}

fn minimize_uint_by_search(
    value: &mut DynSolValue,
    current: U256,
    bits: usize,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    if current <= U256::from(1) {
        return false;
    }

    let mut accepted = current;
    let mut rejected = U256::ZERO;
    let mut changed = false;
    while accepted > rejected + U256::from(1) {
        let candidate: U256 = rejected + ((accepted - rejected) >> 1usize);
        if accept_candidate(value, DynSolValue::Uint(candidate, bits), try_value) {
            accepted = candidate;
            changed = true;
        } else {
            rejected = candidate;
        }
    }

    changed
}

fn minimize_int(
    value: &mut DynSolValue,
    current: I256,
    bits: usize,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    if current != I256::ZERO
        && accept_candidate(value, DynSolValue::Int(I256::ZERO, bits), try_value)
    {
        return true;
    }
    if current.is_negative()
        && current != I256::MINUS_ONE
        && accept_candidate(value, DynSolValue::Int(I256::MINUS_ONE, bits), try_value)
    {
        return true;
    }

    minimize_int_by_search(value, current, bits, try_value)
}

fn minimize_int_by_search(
    value: &mut DynSolValue,
    current: I256,
    bits: usize,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    let mut accepted_abs = current.unsigned_abs();
    if accepted_abs <= U256::from(1) {
        return false;
    }

    let mut rejected_abs = U256::ZERO;
    let mut changed = false;
    while accepted_abs > rejected_abs + U256::from(1) {
        let candidate_abs: U256 = rejected_abs + ((accepted_abs - rejected_abs) >> 1usize);
        let candidate = if current.is_negative() {
            I256::from_raw(candidate_abs.wrapping_neg())
        } else {
            I256::from_raw(candidate_abs)
        };
        if accept_candidate(value, DynSolValue::Int(candidate, bits), try_value) {
            accepted_abs = candidate_abs;
            changed = true;
        } else {
            rejected_abs = candidate_abs;
        }
    }

    changed
}

fn minimize_address(
    value: &mut DynSolValue,
    current: Address,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    for candidate in address_candidates(current) {
        if accept_candidate(value, DynSolValue::Address(candidate), try_value) {
            return true;
        }
    }
    false
}

fn address_candidates(current: Address) -> Vec<Address> {
    if current == Address::ZERO {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    candidates.push(Address::ZERO);

    let deadbeef = Address::from_word(B256::from(U256::from(0xdeadbeefu64)));
    if current != deadbeef {
        candidates.push(deadbeef);
    }

    let bytes = current.into_array();
    for idx in 0..bytes.len() {
        if bytes[idx] == 0 {
            continue;
        }
        let mut candidate = bytes;
        candidate[idx] = 0;
        candidates.push(Address::from(candidate));
    }

    candidates
}

fn minimize_fixed_bytes(
    value: &mut DynSolValue,
    current: B256,
    size: usize,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    if current != B256::ZERO
        && accept_candidate(value, DynSolValue::FixedBytes(B256::ZERO, size), try_value)
    {
        return true;
    }

    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(current.as_slice());
    for idx in (0..size.min(bytes.len())).rev() {
        if bytes[idx] == 0 {
            continue;
        }
        let mut candidate = bytes;
        candidate[idx] = 0;
        if accept_candidate(value, DynSolValue::FixedBytes(B256::from(candidate), size), try_value)
        {
            return true;
        }
    }
    false
}

fn minimize_bytes(
    value: &mut DynSolValue,
    current: Vec<u8>,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    for len in 0..current.len() {
        if accept_candidate(value, DynSolValue::Bytes(current[..len].to_vec()), try_value) {
            return true;
        }
    }
    if try_delete_vec_range(&current, |candidate| {
        accept_candidate(value, DynSolValue::Bytes(candidate), try_value)
    }) {
        return true;
    }
    if try_slice_vec_range(&current, |candidate| {
        accept_candidate(value, DynSolValue::Bytes(candidate), try_value)
    }) {
        return true;
    }

    for idx in (0..current.len()).rev() {
        if current[idx] == 0 {
            continue;
        }
        let mut candidate = current.clone();
        candidate[idx] = 0;
        if accept_candidate(value, DynSolValue::Bytes(candidate), try_value) {
            return true;
        }
    }
    false
}

fn minimize_string(
    value: &mut DynSolValue,
    current: String,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    for len in 0..current.len() {
        if current.is_char_boundary(len)
            && accept_candidate(value, DynSolValue::String(current[..len].to_string()), try_value)
        {
            return true;
        }
    }
    if try_delete_string_range(&current, |candidate| {
        accept_candidate(value, DynSolValue::String(candidate), try_value)
    }) {
        return true;
    }
    if try_slice_string_range(&current, |candidate| {
        accept_candidate(value, DynSolValue::String(candidate), try_value)
    }) {
        return true;
    }

    let current_bytes = current.as_bytes();
    for idx in (0..current_bytes.len()).rev() {
        if current_bytes[idx] == 0 {
            continue;
        }
        let mut candidate = current_bytes.to_vec();
        candidate[idx] = 0;
        if let Ok(candidate) = String::from_utf8(candidate)
            && accept_candidate(value, DynSolValue::String(candidate), try_value)
        {
            return true;
        }
    }
    false
}

fn minimize_array_len(
    value: &mut DynSolValue,
    current: &mut Vec<DynSolValue>,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    for len in 0..current.len() {
        let candidate = DynSolValue::Array(current[..len].to_vec());
        if accept_candidate(value, candidate, try_value) {
            current.truncate(len);
            return true;
        }
    }
    if try_delete_vec_range(current, |candidate| {
        accept_candidate(value, DynSolValue::Array(candidate), try_value)
    }) {
        return true;
    }
    if try_slice_vec_range(current, |candidate| {
        accept_candidate(value, DynSolValue::Array(candidate), try_value)
    }) {
        return true;
    }
    false
}

fn try_delete_vec_range<T: Clone>(
    current: &[T],
    mut try_candidate: impl FnMut(Vec<T>) -> bool,
) -> bool {
    for range_len in deletion_lengths(current.len()) {
        for start in 0..=current.len() - range_len {
            let mut candidate = Vec::with_capacity(current.len() - range_len);
            candidate.extend_from_slice(&current[..start]);
            candidate.extend_from_slice(&current[start + range_len..]);
            if try_candidate(candidate) {
                return true;
            }
        }
    }
    false
}

fn try_delete_string_range(current: &str, mut try_candidate: impl FnMut(String) -> bool) -> bool {
    let mut boundaries = current.char_indices().map(|(idx, _)| idx).collect::<Vec<_>>();
    boundaries.push(current.len());

    for range_len in deletion_lengths(boundaries.len().saturating_sub(1)) {
        for start_idx in 0..=boundaries.len() - range_len - 1 {
            let start = boundaries[start_idx];
            let end = boundaries[start_idx + range_len];
            let mut candidate = String::with_capacity(current.len() - (end - start));
            candidate.push_str(&current[..start]);
            candidate.push_str(&current[end..]);
            if try_candidate(candidate) {
                return true;
            }
        }
    }
    false
}

fn try_slice_vec_range<T: Clone>(
    current: &[T],
    mut try_candidate: impl FnMut(Vec<T>) -> bool,
) -> bool {
    for len in 1..current.len() {
        for start in 1..=current.len() - len {
            let candidate = current[start..start + len].to_vec();
            if try_candidate(candidate) {
                return true;
            }
        }
    }
    false
}

fn try_slice_string_range(current: &str, mut try_candidate: impl FnMut(String) -> bool) -> bool {
    let mut boundaries = current.char_indices().map(|(idx, _)| idx).collect::<Vec<_>>();
    boundaries.push(current.len());
    let char_len = boundaries.len().saturating_sub(1);

    for len in 1..char_len {
        for start_idx in 1..=char_len - len {
            let start = boundaries[start_idx];
            let end = boundaries[start_idx + len];
            if try_candidate(current[start..end].to_string()) {
                return true;
            }
        }
    }
    false
}

fn deletion_lengths(len: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }

    let mut lengths = Vec::new();
    let mut range_len = len;
    while range_len > 0 {
        lengths.push(range_len);
        range_len /= 2;
    }
    lengths.sort_unstable();
    lengths.dedup();
    lengths.reverse();
    lengths
}

fn minimize_elements(
    elements: &mut [DynSolValue],
    rebuild: impl Fn(&[DynSolValue]) -> DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> Option<DynSolValue> {
    for idx in 0..elements.len() {
        let mut element = elements[idx].clone();
        let changed = minimize_value(&mut element, &mut |candidate| {
            let mut candidate_elements = elements.to_vec();
            candidate_elements[idx] = candidate.clone();
            try_value(&rebuild(&candidate_elements))
        });
        if changed {
            elements[idx] = element;
            return Some(rebuild(elements));
        }
    }
    None
}

fn minimize_elements_batch(
    elements: &mut Vec<DynSolValue>,
    rebuild: impl Fn(&[DynSolValue]) -> DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> Option<DynSolValue> {
    let simple_elements = elements.iter().cloned().map(minimally_simple_value).collect::<Vec<_>>();
    if simple_elements == *elements {
        return None;
    }
    let candidate = rebuild(&simple_elements);
    try_value(&candidate).then(|| {
        *elements = simple_elements;
        candidate
    })
}

fn minimize_element_subsets(
    elements: &mut Vec<DynSolValue>,
    rebuild: impl Fn(&[DynSolValue]) -> DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> Option<DynSolValue> {
    let simple_elements = elements.iter().cloned().map(minimally_simple_value).collect::<Vec<_>>();
    let shrinkable_idxs = elements
        .iter()
        .zip(&simple_elements)
        .enumerate()
        .filter_map(|(idx, (current, simple))| (current != simple).then_some(idx))
        .collect::<Vec<_>>();
    if shrinkable_idxs.len() < 2 {
        return None;
    }

    for subset_size in subset_sizes(shrinkable_idxs.len()) {
        let mut search = ElementSubsetSearch {
            elements,
            simple_elements: &simple_elements,
            shrinkable_idxs: &shrinkable_idxs,
            rebuild: &rebuild,
            try_value,
        };
        let mut subset = Vec::with_capacity(subset_size);
        if let Some(candidate_elements) =
            try_element_subset(&mut search, subset_size, 0, &mut subset)
        {
            *elements = candidate_elements.clone();
            return Some(rebuild(&candidate_elements));
        }
    }
    None
}

fn subset_sizes(shrinkable_len: usize) -> Vec<usize> {
    let mut sizes = Vec::new();
    for subset_size in 2..shrinkable_len {
        if bounded_combination_count(shrinkable_len, subset_size, MAX_SUBSET_CANDIDATES_PER_PASS)
            .is_some()
        {
            sizes.push(subset_size);
        }
    }
    sizes
}

fn bounded_combination_count(n: usize, k: usize, max: usize) -> Option<usize> {
    if k > n {
        return None;
    }
    let k = k.min(n - k);
    let mut count = 1usize;
    for step in 1..=k {
        count = count.checked_mul(n + 1 - step)?;
        count /= step;
        if count > max {
            return None;
        }
    }
    Some(count)
}

struct ElementSubsetSearch<'a, R>
where
    R: Fn(&[DynSolValue]) -> DynSolValue,
{
    elements: &'a [DynSolValue],
    simple_elements: &'a [DynSolValue],
    shrinkable_idxs: &'a [usize],
    rebuild: &'a R,
    try_value: &'a mut dyn FnMut(&DynSolValue) -> bool,
}

fn try_element_subset<R>(
    search: &mut ElementSubsetSearch<'_, R>,
    subset_size: usize,
    start: usize,
    subset: &mut Vec<usize>,
) -> Option<Vec<DynSolValue>>
where
    R: Fn(&[DynSolValue]) -> DynSolValue,
{
    if subset.len() == subset_size {
        let mut candidate_elements = search.elements.to_vec();
        for idx in subset.iter().copied() {
            candidate_elements[idx] = search.simple_elements[idx].clone();
        }
        if (search.try_value)(&(search.rebuild)(&candidate_elements)) {
            return Some(candidate_elements);
        }
        return None;
    }

    let remaining = subset_size - subset.len();
    for choice_idx in start..=search.shrinkable_idxs.len() - remaining {
        subset.push(search.shrinkable_idxs[choice_idx]);
        if let Some(candidate) = try_element_subset(search, subset_size, choice_idx + 1, subset) {
            return Some(candidate);
        }
        subset.pop();
    }
    None
}

fn accept_candidate(
    value: &mut DynSolValue,
    candidate: DynSolValue,
    try_value: &mut dyn FnMut(&DynSolValue) -> bool,
) -> bool {
    if *value == candidate {
        return false;
    }
    if try_value(&candidate) {
        *value = candidate;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_json_abi::JsonAbi;

    fn call(function: &Function, args: Vec<DynSolValue>) -> SymbolicCounterexampleCall {
        SymbolicCounterexampleCall {
            warp: None,
            roll: None,
            sender: Address::ZERO,
            target: Address::repeat_byte(0x11),
            calldata: Bytes::from(function.abi_encode_input(&args).unwrap()),
            value: Some(U256::ZERO),
            contract_name: Some("Target".to_string()),
            function_name: Some(function.name.clone()),
            signature: Some(function.signature()),
            args: Some(foundry_common::fmt::format_tokens(&args).format(", ").to_string()),
            raw_args: Some(foundry_common::fmt::format_tokens_raw(&args).format(", ").to_string()),
        }
    }

    fn decoded(function: &Function, call: &SymbolicCounterexampleCall) -> Vec<DynSolValue> {
        function.abi_decode_input(&call.calldata[4..]).unwrap()
    }

    #[test]
    fn minimizes_common_abi_values_with_replay_predicate() {
        let abi =
            JsonAbi::parse(["function check(uint256,address,bytes,string,uint256[]) external"])
                .unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(
            function,
            vec![
                DynSolValue::Uint(U256::from(0xff), 256),
                DynSolValue::Address(Address::from([0xaa; 20])),
                DynSolValue::Bytes(vec![0x99, 0x42, 0x88]),
                DynSolValue::String("abc".to_string()),
                DynSolValue::Array(vec![
                    DynSolValue::Uint(U256::from(0), 256),
                    DynSolValue::Uint(U256::from(7), 256),
                    DynSolValue::Uint(U256::from(9), 256),
                ]),
            ],
        );

        let minimized =
            minimize_single_call_counterexample(function, &start, &[], |candidate| {
                let args = decoded(function, candidate);
                matches!(&args[0], DynSolValue::Uint(value, _) if *value & U256::from(0x2a) == U256::from(0x2a))
                    && matches!(&args[1], DynSolValue::Address(address) if address.as_slice()[19] == 0xaa)
                    && matches!(&args[2], DynSolValue::Bytes(bytes) if bytes.get(1) == Some(&0x42))
                    && matches!(&args[3], DynSolValue::String(value) if value.starts_with('a'))
                    && matches!(&args[4], DynSolValue::Array(values) if values.iter().any(|value| matches!(value, DynSolValue::Uint(uint, _) if *uint == U256::from(7))))
            })
            .unwrap();

        let args = decoded(function, &minimized.minimized_call);
        assert_eq!(args[0], DynSolValue::Uint(U256::from(0x2a), 256));
        assert_eq!(
            args[1],
            DynSolValue::Address(Address::from([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xaa,
            ]))
        );
        assert_eq!(args[2], DynSolValue::Bytes(vec![0, 0x42]));
        assert_eq!(args[3], DynSolValue::String("a".to_string()));
        assert_eq!(args[4], DynSolValue::Array(vec![DynSolValue::Uint(U256::from(7), 256)]));
        assert!(minimized.changed());
        assert!(minimized.attempts > minimized.accepted);
        assert!(minimized.accepted > 0);
    }

    #[test]
    fn minimizes_with_echidna_style_range_deletion_and_numeric_lowering() {
        let abi =
            JsonAbi::parse(["function check(uint256,int256,bytes,string,uint256[]) external"])
                .unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(
            function,
            vec![
                DynSolValue::Uint(U256::from(100), 256),
                DynSolValue::Int(I256::try_from(-100).unwrap(), 256),
                DynSolValue::Bytes(vec![1, 2, 3, 0x42, 4]),
                DynSolValue::String("abcZ".to_string()),
                DynSolValue::Array(vec![
                    DynSolValue::Uint(U256::from(1), 256),
                    DynSolValue::Uint(U256::from(2), 256),
                    DynSolValue::Uint(U256::from(7), 256),
                    DynSolValue::Uint(U256::from(3), 256),
                ]),
            ],
        );

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[0], DynSolValue::Uint(value, _) if *value > U256::from(42))
                && matches!(&args[1], DynSolValue::Int(value, _) if *value < I256::try_from(-42).unwrap())
                && matches!(&args[2], DynSolValue::Bytes(bytes) if bytes.contains(&0x42))
                && matches!(&args[3], DynSolValue::String(value) if value.contains('Z'))
                && matches!(&args[4], DynSolValue::Array(values) if values.iter().any(|value| matches!(value, DynSolValue::Uint(uint, _) if *uint == U256::from(7))))
        })
        .unwrap();

        let args = decoded(function, &minimized.minimized_call);
        assert_eq!(args[0], DynSolValue::Uint(U256::from(43), 256));
        assert_eq!(args[1], DynSolValue::Int(I256::try_from(-43).unwrap(), 256));
        assert_eq!(args[2], DynSolValue::Bytes(vec![0x42]));
        assert_eq!(args[3], DynSolValue::String("Z".to_string()));
        assert_eq!(args[4], DynSolValue::Array(vec![DynSolValue::Uint(U256::from(7), 256)]));
        assert!(minimized.changed());
        assert!(minimized.accepted > 0);
    }

    #[test]
    fn matches_echidna_uint8_threshold_shrink_result() {
        let abi = JsonAbi::parse(["function check(uint8) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(function, vec![DynSolValue::Uint(U256::from(246), 8)]);

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[0], DynSolValue::Uint(value, 8) if *value > U256::from(42))
        })
        .unwrap();

        assert_eq!(
            decoded(function, &minimized.minimized_call),
            vec![DynSolValue::Uint(U256::from(43), 8)]
        );
        assert!(minimized.attempts < MAX_MINIMIZATION_ATTEMPTS);
    }

    #[test]
    fn matches_echidna_contiguous_slice_examples() {
        let bytes_abi = JsonAbi::parse(["function check(bytes) external"]).unwrap();
        let bytes_function = bytes_abi.functions().next().unwrap();
        let bytes_start =
            call(bytes_function, vec![DynSolValue::Bytes(vec![0x99, 0x41, 0x42, 0x88])]);
        let bytes_minimized =
            minimize_single_call_counterexample(bytes_function, &bytes_start, &[], |candidate| {
                decoded(bytes_function, candidate) == vec![DynSolValue::Bytes(vec![0x41, 0x42])]
            })
            .unwrap();
        assert_eq!(
            decoded(bytes_function, &bytes_minimized.minimized_call),
            vec![DynSolValue::Bytes(vec![0x41, 0x42])]
        );

        let string_abi = JsonAbi::parse(["function check(string) external"]).unwrap();
        let string_function = string_abi.functions().next().unwrap();
        let string_start = call(string_function, vec![DynSolValue::String("xOKy".to_string())]);
        let string_minimized =
            minimize_single_call_counterexample(string_function, &string_start, &[], |candidate| {
                decoded(string_function, candidate) == vec![DynSolValue::String("OK".to_string())]
            })
            .unwrap();
        assert_eq!(
            decoded(string_function, &string_minimized.minimized_call),
            vec![DynSolValue::String("OK".to_string())]
        );

        let array_abi = JsonAbi::parse(["function check(uint256[]) external"]).unwrap();
        let array_function = array_abi.functions().next().unwrap();
        let array_start = call(
            array_function,
            vec![DynSolValue::Array(vec![
                DynSolValue::Uint(U256::from(9), 256),
                DynSolValue::Uint(U256::from(4), 256),
                DynSolValue::Uint(U256::from(2), 256),
                DynSolValue::Uint(U256::from(8), 256),
            ])],
        );
        let array_minimized =
            minimize_single_call_counterexample(array_function, &array_start, &[], |candidate| {
                let args = decoded(array_function, candidate);
                matches!(&args[0], DynSolValue::Array(values) if values == &[
                    DynSolValue::Uint(U256::from(4), 256),
                    DynSolValue::Uint(U256::from(2), 256),
                ])
            })
            .unwrap();
        assert_eq!(
            decoded(array_function, &array_minimized.minimized_call),
            vec![DynSolValue::Array(vec![
                DynSolValue::Uint(U256::from(4), 256),
                DynSolValue::Uint(U256::from(2), 256),
            ])]
        );
    }

    #[test]
    fn matches_echidna_address_deadbeef_and_bool_examples() {
        let deadbeef = Address::from_word(B256::from(U256::from(0xdeadbeefu64)));

        let address_abi = JsonAbi::parse(["function check(address) external"]).unwrap();
        let address_function = address_abi.functions().next().unwrap();
        let address_start =
            call(address_function, vec![DynSolValue::Address(Address::from([0xaa; 20]))]);
        let address_minimized = minimize_single_call_counterexample(
            address_function,
            &address_start,
            &[],
            |candidate| {
                let args = decoded(address_function, candidate);
                matches!(&args[..], [DynSolValue::Address(address)] if *address == deadbeef)
            },
        )
        .unwrap();
        assert_eq!(
            decoded(address_function, &address_minimized.minimized_call),
            vec![DynSolValue::Address(deadbeef)]
        );

        let bool_abi = JsonAbi::parse(["function check(bool) external"]).unwrap();
        let bool_function = bool_abi.functions().next().unwrap();
        let bool_start = call(bool_function, vec![DynSolValue::Bool(true)]);
        let bool_minimized =
            minimize_single_call_counterexample(bool_function, &bool_start, &[], |candidate| {
                decoded(bool_function, candidate) == vec![DynSolValue::Bool(false)]
            })
            .unwrap();
        assert_eq!(
            decoded(bool_function, &bool_minimized.minimized_call),
            vec![DynSolValue::Bool(false)]
        );
    }

    #[test]
    fn minimizes_correlated_multi_arg_slice_examples() {
        let abi = JsonAbi::parse(["function check(bytes,string) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(
            function,
            vec![
                DynSolValue::Bytes(vec![0x99, 0x41, 0x42, 0x88]),
                DynSolValue::String("xOKy".to_string()),
            ],
        );

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[..], [
                DynSolValue::Bytes(bytes),
                DynSolValue::String(string),
            ] if bytes == &[0x41, 0x42] && string.contains("OK"))
        })
        .unwrap();

        assert_eq!(
            decoded(function, &minimized.minimized_call),
            vec![DynSolValue::Bytes(vec![0x41, 0x42]), DynSolValue::String("OK".to_string()),]
        );
        assert!(minimized.changed());
    }

    #[test]
    fn minimizes_correlated_top_level_abi_value_subsets() {
        let abi = JsonAbi::parse(["function check(uint256,uint256,bytes) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(
            function,
            vec![
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Bytes(vec![0x99, 0x42, 0x88]),
            ],
        );

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[..], [
                DynSolValue::Uint(left, _),
                DynSolValue::Uint(right, _),
                DynSolValue::Bytes(bytes),
            ] if left.is_zero() && right.is_zero() && bytes.contains(&0x42))
        })
        .unwrap();

        assert_eq!(
            decoded(function, &minimized.minimized_call),
            vec![
                DynSolValue::Uint(U256::ZERO, 256),
                DynSolValue::Uint(U256::ZERO, 256),
                DynSolValue::Bytes(vec![0x42]),
            ]
        );
        assert!(minimized.accepted > 0);
    }

    #[test]
    fn minimizes_correlated_nested_abi_value_subsets() {
        let abi = JsonAbi::parse(["function check((uint256,uint256,bytes)) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let start = call(
            function,
            vec![DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Uint(U256::from(1), 256),
                DynSolValue::Bytes(vec![0x99, 0x42, 0x88]),
            ])],
        );

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[0], DynSolValue::Tuple(values)
                if matches!(&values[..], [
                    DynSolValue::Uint(left, _),
                    DynSolValue::Uint(right, _),
                    DynSolValue::Bytes(bytes),
                ] if left.is_zero() && right.is_zero() && bytes.contains(&0x42)))
        })
        .unwrap();

        assert_eq!(
            decoded(function, &minimized.minimized_call),
            vec![DynSolValue::Tuple(vec![
                DynSolValue::Uint(U256::ZERO, 256),
                DynSolValue::Uint(U256::ZERO, 256),
                DynSolValue::Bytes(vec![0x42]),
            ]),]
        );
        assert!(minimized.accepted > 0);
    }

    #[test]
    fn minimizes_correlated_args_and_call_env() {
        let abi = JsonAbi::parse(["function check(uint256,uint256) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let mut start = call(
            function,
            vec![DynSolValue::Uint(U256::from(1), 256), DynSolValue::Uint(U256::from(1), 256)],
        );
        start.sender = Address::from([0xaa; 20]);
        start.value = Some(U256::from(100));
        start.warp = Some(U256::from(50));
        start.roll = Some(U256::from(10));

        let configured_senders = [Address::ZERO, start.sender];
        let minimized =
            minimize_single_call_counterexample(function, &start, &configured_senders, |candidate| {
                let args = decoded(function, candidate);
                matches!(&args[..], [DynSolValue::Uint(left, _), DynSolValue::Uint(right, _)] if left.is_zero() && right.is_zero())
                    && candidate.value.is_some_and(|value| value > U256::from(42))
                    && candidate.warp.is_some_and(|warp| warp > U256::from(4))
                    && candidate.roll.is_some_and(|roll| roll > U256::from(2))
                    && (candidate.sender == Address::ZERO || candidate.sender.as_slice()[19] == 0xaa)
            })
            .unwrap();

        let args = decoded(function, &minimized.minimized_call);
        assert_eq!(
            args,
            vec![DynSolValue::Uint(U256::ZERO, 256), DynSolValue::Uint(U256::ZERO, 256)]
        );
        assert_eq!(minimized.minimized_call.sender, Address::ZERO);
        assert_eq!(minimized.minimized_call.value, Some(U256::from(43)));
        assert_eq!(minimized.minimized_call.warp, Some(U256::from(5)));
        assert_eq!(minimized.minimized_call.roll, Some(U256::from(3)));
        assert!(minimized.changed());
        assert!(minimized.accepted > 0);
    }

    #[test]
    fn leaves_already_minimal_counterexample_replayable() {
        let abi = JsonAbi::parse(["function check(int256,bool,bytes3) external"]).unwrap();
        let function = abi.functions().next().unwrap();
        let mut fixed_bytes = [0u8; 32];
        fixed_bytes[2] = 0x42;
        let start = call(
            function,
            vec![
                DynSolValue::Int(I256::MINUS_ONE, 256),
                DynSolValue::Bool(false),
                DynSolValue::FixedBytes(B256::from(fixed_bytes), 3),
            ],
        );

        let minimized = minimize_single_call_counterexample(function, &start, &[], |candidate| {
            let args = decoded(function, candidate);
            matches!(&args[0], DynSolValue::Int(value, _) if *value == I256::MINUS_ONE)
                && matches!(&args[1], DynSolValue::Bool(false))
                && matches!(&args[2], DynSolValue::FixedBytes(bytes, 3) if bytes[2] == 0x42)
        })
        .unwrap();

        assert_eq!(decoded(function, &minimized.minimized_call), decoded(function, &start));
        assert!(!minimized.changed());
        assert_eq!(minimized.accepted, 0);
    }
}
