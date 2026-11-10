use super::*;
use evm2::interpreter::gas::CALL_STIPEND;

/// Order of the secp256k1 curve, the exclusive bound of a valid signature scalar.
const SECP256K1_ORDER: U256 =
    U256::from_be_bytes(hex!("fffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141"));

pub(crate) fn precompile_number(address: Address) -> Option<u8> {
    let bytes = address.as_slice();
    if bytes[..PRECOMPILE_ADDRESS_LEADING_ZEROS].iter().any(|byte| *byte != 0) {
        return None;
    }
    match bytes[PRECOMPILE_ADDRESS_LEADING_ZEROS] {
        1..=10 => Some(bytes[PRECOMPILE_ADDRESS_LEADING_ZEROS]),
        _ => None,
    }
}

pub(crate) fn precompile_number_for_spec(address: Address, spec_id: SpecId) -> Option<u8> {
    match precompile_number(address)? {
        5..=8 if spec_id < SpecId::BYZANTIUM => None,
        9 if spec_id < SpecId::ISTANBUL => None,
        10 if spec_id < SpecId::CANCUN => None,
        number => Some(number),
    }
}

/// Returns the canonical precompiles active under `spec_id`.
pub(crate) fn active_precompiles(spec_id: SpecId) -> Precompiles {
    Precompiles::base(spec_id)
}

/// Returns whether `address` is a canonical precompile active under `spec_id`.
pub(crate) fn is_precompile_for_spec(address: Address, spec_id: SpecId) -> bool {
    active_precompiles(spec_id).as_map().contains(&address)
}

/// Returns the gas a call gives a precompile in a transaction with `gas_limit` gas.
///
/// A call forwards its gas operand, at most 63/64 of the gas left (EIP-150), plus the value
/// transfer stipend when it sends value. A symbolic operand counts as the most it can forward.
pub(crate) fn precompile_call_gas(
    gas_limit: u64,
    gas_operand: Option<U256>,
    sends_value: bool,
) -> u64 {
    let max_forwarded = gas_limit - gas_limit / 64;
    let forwarded =
        gas_operand.map_or(max_forwarded, |gas| gas.saturating_to::<u64>().min(max_forwarded));
    if sends_value { forwarded.saturating_add(CALL_STIPEND.into()) } else { forwarded }
}

/// Executes a canonical precompile on concrete input with at most `gas` gas.
///
/// Returns `None` when the precompile fails, including when it runs out of gas.
pub(crate) fn execute_precompile(
    cx: &mut SymCx,
    address: Address,
    input: &[u8],
    spec_id: SpecId,
    gas: u64,
) -> Result<Option<SymReturnData>, SymbolicError> {
    if !is_precompile_for_spec(address, spec_id) {
        return Err(SymbolicError::Unsupported("unsupported precompile"));
    }
    // evm2's precompile table runs these through an EVM; select the same input-only
    // implementations the base table registers for `spec_id`.
    let run = match u16::from_be_bytes([address[18], address[19]]) {
        0x01 => secp256k1::run,
        0x02 => hash::run_sha256,
        0x03 => hash::run_ripemd160,
        0x04 => identity::run,
        0x05 if spec_id.enables(SpecId::OSAKA) => modexp::run_osaka,
        0x05 if spec_id.enables(SpecId::BERLIN) => modexp::run_berlin,
        0x05 => modexp::run_byzantium,
        0x06 if spec_id.enables(SpecId::ISTANBUL) => bn254::add::run_istanbul,
        0x06 => bn254::add::run_byzantium,
        0x07 if spec_id.enables(SpecId::ISTANBUL) => bn254::mul::run_istanbul,
        0x07 => bn254::mul::run_byzantium,
        0x08 if spec_id.enables(SpecId::ISTANBUL) => bn254::pair::run_istanbul,
        0x08 => bn254::pair::run_byzantium,
        0x09 => blake2::run,
        0x0a => kzg_point_evaluation::run,
        0x0b => bls12_381::g1_add::run,
        0x0c => bls12_381::g1_msm::run,
        0x0d => bls12_381::g2_add::run,
        0x0e => bls12_381::g2_msm::run,
        0x0f => bls12_381::pairing::run,
        0x10 => bls12_381::map_fp_to_g1::run,
        0x11 => bls12_381::map_fp2_to_g2::run,
        0x100 => secp256r1::run_osaka,
        _ => return Err(SymbolicError::Unsupported("unsupported precompile")),
    };

    match run(input, &mut GasTracker::new(gas)) {
        Ok(output) => Ok(Some(SymReturnData::from_concrete_bytes(cx, output.bytes().to_vec()))),
        Err(_) => Ok(None),
    }
}

pub(crate) fn execute_symbolic_precompile(
    cx: &mut SymCx,
    address: Address,
    input: SymBytes,
    input_len: SymExpr,
    spec_id: SpecId,
    gas: u64,
    modexp_lengths: Option<ModexpLengthRanges>,
) -> Result<Option<SymReturnData>, SymbolicError> {
    match precompile_number_for_spec(address, spec_id) {
        Some(1) => {
            // ECRECOVER costs 3000 gas for any input.
            if gas < 3_000 {
                return Ok(None);
            }
            // ECRECOVER ignores trailing bytes and pads short input with zeros.
            let input = SymBytes::sized(cx, input, input_len, 128);
            if let Ok(input) = input.concrete_bytes(cx, "symbolic ecrecover input") {
                return execute_precompile(cx, address, &input, spec_id, gas);
            }
            let valid_v = ecrecover_signature_well_formed(cx, &input);
            if valid_v.as_const() == Some(false) {
                return Ok(Some(SymReturnData::empty(cx)));
            }

            let input = input.materialize(cx);
            let input_len = SymExpr::constant(cx, U256::from(128));
            let word = symbolic_hash_word_with_len(cx, "ecrecover", input, input_len);
            // Recovery may fail even with a valid v. Use an otherwise discarded byte of the
            // opaque word so this choice is independent of the low 160-bit recovered address.
            let recovered = byte_word(cx, U256::ZERO, word.clone()).nonzero_bool(cx);
            let recovered = SymBoolExpr::and(cx, vec![valid_v, recovered]);
            let full_len = SymExpr::constant(cx, U256::from(32));
            let empty_len = SymExpr::zero(cx);
            let len = SymExpr::ite(cx, recovered, full_len, empty_len);
            let mut bytes = vec![SymExpr::zero(cx); 12];
            bytes.extend((12..32).map(|idx| byte_word(cx, U256::from(idx), word.clone())));
            let bytes = SymBytes::exprs(cx, bytes);
            Ok(Some(SymReturnData { len_word: len, bytes }))
        }
        Some(2) => {
            // SHA256 costs 60 gas plus 12 per word.
            if !linear_precompile_gas_fits(&input, &input_len, gas, 60, 12)? {
                return Ok(None);
            }
            let input = input.materialize(cx);
            let word = symbolic_hash_word_with_len(cx, "sha256", input, input_len);
            let bytes = word.into_byte_exprs(cx);
            Ok(Some(SymReturnData::from_byte_exprs(cx, bytes)))
        }
        Some(3) => {
            // RIPEMD160 costs 600 gas plus 120 per word.
            if !linear_precompile_gas_fits(&input, &input_len, gas, 600, 120)? {
                return Ok(None);
            }
            let input = input.materialize(cx);
            let word = symbolic_hash_word_with_len(cx, "ripemd160", input, input_len);
            let mut bytes = vec![SymExpr::zero(cx); 12];
            bytes.extend((12..32).map(|idx| byte_word(cx, U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_byte_exprs(cx, bytes)))
        }
        Some(4) => {
            // Identity costs 15 gas plus 3 per word.
            if !linear_precompile_gas_fits(&input, &input_len, gas, 15, 3)? {
                return Ok(None);
            }
            Ok(Some(SymReturnData { len_word: input_len, bytes: input }))
        }
        Some(5) => symbolic_modexp_precompile(cx, &input, input_len, spec_id, gas, modexp_lengths),
        Some(6) => {
            let input_len = input_len.as_usize_or("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if (0..input_len).any(|idx| input.byte(cx, idx).as_const().is_none()) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            // BN254 addition costs 150 gas from Istanbul (EIP-1108) and 500 before.
            let cost = if spec_id.enables(SpecId::ISTANBUL) { 150 } else { 500 };
            if cost > gas {
                return Ok(None);
            }
            Ok(Some(symbolic_fixed_len_precompile_output(cx, "bn254_add", &input, input_len, 64)))
        }
        Some(7) => {
            let input_len = input_len.as_usize_or("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if (0..input_len).any(|idx| input.byte(cx, idx).as_const().is_none()) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            // BN254 multiplication costs 6000 gas from Istanbul (EIP-1108) and 40000 before.
            let cost = if spec_id.enables(SpecId::ISTANBUL) { 6_000 } else { 40_000 };
            if cost > gas {
                return Ok(None);
            }
            Ok(Some(symbolic_fixed_len_precompile_output(cx, "bn254_mul", &input, input_len, 64)))
        }
        Some(8) => {
            let input_len = input_len.as_usize_or("symbolic precompile input")?;
            if input_len % 192 != 0 {
                return Ok(None);
            }
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if (0..input_len).any(|idx| input.byte(cx, idx).as_const().is_none()) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            // BN254 pairing costs 45000 gas plus 34000 per pair from Istanbul (EIP-1108), and
            // 100000 plus 80000 per pair before.
            let (base, per_pair) = if spec_id.enables(SpecId::ISTANBUL) {
                (45_000, 34_000)
            } else {
                (100_000, 80_000)
            };
            if (input_len / 192) as u64 * per_pair + base > gas {
                return Ok(None);
            }
            Ok(Some(symbolic_fixed_len_precompile_output(
                cx,
                "bn254_pairing",
                &input,
                input_len,
                32,
            )))
        }
        Some(9) => {
            let input_len = input_len.as_usize_or("symbolic precompile input")?;
            if input_len != 213 {
                return Ok(None);
            }
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            // BLAKE2f costs one gas per round, from the first four input bytes.
            let (min_rounds, max_rounds) = symbolic_input_word_range(cx, &input, input_len, 0, 4);
            match precompile_gas_fits(gas, min_rounds.saturating_to(), max_rounds.saturating_to()) {
                Some(true) => {}
                Some(false) => return Ok(None),
                None => {
                    return Err(SymbolicError::Unsupported("symbolic blake2f precompile gas"));
                }
            }
            let flag = input.byte(cx, 212);
            match flag.as_const() {
                Some(flag) if flag.is_zero() || flag == U256::ONE => {}
                Some(_) => return Ok(None),
                None => {
                    return Err(SymbolicError::Unsupported(
                        "symbolic blake2f precompile final flag not modeled",
                    ));
                }
            }
            Ok(Some(symbolic_fixed_len_precompile_output(cx, "blake2f", &input, input_len, 64)))
        }
        Some(10) => Err(SymbolicError::Unsupported("KZG handled by execute_kzg_precompile_call")),
        _ => {
            let input_len = input_len.as_usize_or("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            let input = input
                .slice_concrete(cx, 0, input_len)
                .concrete_bytes(cx, "symbolic precompile input")?;
            execute_precompile(cx, address, &input, spec_id, gas)
        }
    }
}

/// Returns whether a 128-byte ECRECOVER input is a well-formed signature: `v` is 27 or 28 and `r`
/// and `s` are in `[1, n)`. The precompile returns no data otherwise.
pub(crate) fn ecrecover_signature_well_formed(cx: &mut SymCx, input: &SymBytes) -> SymBoolExpr {
    let v = input.word_at(cx, 32);
    let v27 = SymBoolExpr::eq_word_const(cx, &v, U256::from(27));
    let v28 = SymBoolExpr::eq_word_const(cx, &v, U256::from(28));
    let mut valid = vec![SymBoolExpr::or(cx, vec![v27, v28])];
    for offset in [64, 96] {
        let scalar = input.word_at(cx, offset);
        valid.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ult, &scalar, SECP256K1_ORDER));
        valid.push(scalar.nonzero_bool(cx));
    }
    SymBoolExpr::and(cx, valid)
}

/// Opaque term algorithm of the modexp precompile, whose outputs may collide.
pub(crate) const MODEXP_ALGORITHM: &str = "modexp";

/// The smallest and largest values of the modexp base, exponent and modulus length headers.
pub(crate) type ModexpLengthRanges = [(u64, u64); 3];

pub(crate) fn symbolic_modexp_precompile(
    cx: &mut SymCx,
    input: &SymBytes,
    input_len: SymExpr,
    spec_id: SpecId,
    gas: u64,
    lengths: Option<ModexpLengthRanges>,
) -> Result<Option<SymReturnData>, SymbolicError> {
    let input_len = input_len.as_usize_or("symbolic precompile input")?;
    if input_len > input.len() {
        return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
    }

    // The cost depends on the lengths and the exponent's leading word, read as evm2 does. Each
    // is bounded by its range, and the cost grows with each of them.
    let [(min_base_len, max_base_len), (min_exp_len, max_exp_len), (min_mod_len, max_mod_len)] =
        lengths.unwrap_or_else(|| {
            [0, 32, 64].map(|offset| precompile_word_byte_range(cx, input, input_len, offset))
        });
    if spec_id.enables(SpecId::OSAKA) {
        // EIP-7823 rejects any length above 1024 bytes.
        let limit = MODEXP_INPUT_SIZE_LIMIT as u64;
        if min_base_len > limit || min_exp_len > limit || min_mod_len > limit {
            return Ok(None);
        }
        if max_base_len > limit || max_exp_len > limit || max_mod_len > limit {
            return Err(SymbolicError::Unsupported("symbolic modexp length header"));
        }
    }
    let (min_exp_highp, max_exp_highp) = if min_base_len == max_base_len
        && let Some(exp_offset) =
            usize::try_from(min_base_len).ok().and_then(|base_len| base_len.checked_add(96))
    {
        let (min, _) = symbolic_input_word_range(
            cx,
            input,
            input_len,
            exp_offset,
            min_exp_len.min(32) as usize,
        );
        let (_, max) = symbolic_input_word_range(
            cx,
            input,
            input_len,
            exp_offset,
            max_exp_len.min(32) as usize,
        );
        (min, max)
    } else {
        // The exponent's offset is symbolic, so its leading word can be any value of its length.
        (U256::ZERO, U256::MAX >> (256 - 8 * max_exp_len.min(32) as usize))
    };
    let min_gas = modexp_gas(spec_id, min_base_len, min_exp_len, min_mod_len, &min_exp_highp);
    let max_gas = modexp_gas(spec_id, max_base_len, max_exp_len, max_mod_len, &max_exp_highp);
    match precompile_gas_fits(gas, min_gas, max_gas) {
        Some(true) => {}
        Some(false) => return Ok(None),
        None => return Err(SymbolicError::Unsupported("symbolic modexp precompile gas")),
    }

    if min_mod_len != max_mod_len {
        return Err(SymbolicError::Unsupported("symbolic modexp output length"));
    }
    let modulus_len = usize::try_from(min_mod_len)
        .ok()
        .filter(|modulus_len| *modulus_len <= 4096)
        .ok_or(SymbolicError::Unsupported("symbolic modexp output length"))?;
    Ok(Some(symbolic_fixed_len_precompile_output(cx, MODEXP_ALGORITHM, input, input_len, modulus_len)))
}

/// Returns the gas the modexp precompile for `spec_id` charges for these lengths and leading
/// exponent word, as EIP-198, EIP-2565 (Berlin) and EIP-7883 (Osaka) define it.
///
/// This mirrors evm2's modexp gas calculation, which evm2 does not export.
fn modexp_gas(spec_id: SpecId, base_len: u64, exp_len: u64, mod_len: u64, exp_highp: &U256) -> u64 {
    let (min_price, multiplier, divisor) = if spec_id.enables(SpecId::OSAKA) {
        (500u64, 16u64, 1u64)
    } else if spec_id.enables(SpecId::BERLIN) {
        (200, 8, 3)
    } else {
        (0, 8, 20)
    };

    let max_len = base_len.max(mod_len);
    let words_squared = || {
        let words = U256::from(max_len.div_ceil(8));
        words * words
    };
    let multiplication_complexity = if spec_id.enables(SpecId::OSAKA) {
        if max_len <= 32 { U256::from(16) } else { words_squared() * U256::from(2) }
    } else if spec_id.enables(SpecId::BERLIN) {
        words_squared()
    } else {
        let x = U256::from(max_len);
        if max_len <= 64 {
            x * x
        } else if max_len <= 1_024 {
            x * x / U256::from(4) + U256::from(96) * x - U256::from(3_072)
        } else {
            x * x / U256::from(16) + U256::from(480) * x - U256::from(199_680)
        }
    };

    let exp_bits = exp_highp.bit_len() as u64;
    let iteration_count = if exp_len <= 32 {
        exp_bits.saturating_sub(1)
    } else {
        multiplier.saturating_mul(exp_len - 32).saturating_add(exp_bits.max(1) - 1)
    };
    let gas = multiplication_complexity * U256::from(iteration_count.max(1)) / U256::from(divisor);
    gas.saturating_to::<u64>().max(min_price)
}

/// Returns whether a precompile that costs `base` plus `per_word` for each 32-byte word of input
/// fits in `gas`.
///
/// A symbolic input length lies between zero and the length of `input`. The precompile is
/// unsupported when whether it fits depends on that length.
fn linear_precompile_gas_fits(
    input: &SymBytes,
    input_len: &SymExpr,
    gas: u64,
    base: u64,
    per_word: u64,
) -> Result<bool, SymbolicError> {
    let (min_len, max_len) = match input_len.as_const() {
        Some(len) => {
            let len = usize::try_from(len).unwrap_or(usize::MAX);
            (len, len)
        }
        None => (0, input.len()),
    };
    let cost = |len: usize| (len as u64).div_ceil(32).saturating_mul(per_word).saturating_add(base);
    precompile_gas_fits(gas, cost(min_len), cost(max_len))
        .ok_or(SymbolicError::Unsupported("symbolic precompile input length gas"))
}

/// Returns whether a precompile whose cost lies between `min_cost` and `max_cost` fits in
/// `gas`, or `None` when that depends on symbolic input.
const fn precompile_gas_fits(gas: u64, min_cost: u64, max_cost: u64) -> Option<bool> {
    if max_cost <= gas {
        Some(true)
    } else if min_cost > gas {
        Some(false)
    } else {
        None
    }
}

/// Returns the smallest and largest big-endian values of the `len` input bytes at `offset`,
/// taking each symbolic byte as `0x00` and `0xff` and each byte past `input_len` as zero.
fn symbolic_input_word_range(
    cx: &mut SymCx,
    input: &SymBytes,
    input_len: usize,
    offset: usize,
    len: usize,
) -> (U256, U256) {
    let (mut min, mut max) = (U256::ZERO, U256::ZERO);
    for idx in offset..offset.saturating_add(len) {
        let byte = if idx < input_len { input.byte(cx, idx).as_const() } else { Some(U256::ZERO) };
        min = (min << 8) | byte.unwrap_or_default();
        max = (max << 8) | byte.unwrap_or(U256::from(0xff));
    }
    (min, max)
}

/// Returns the smallest and largest values, saturated to `u64`, of the input word at `offset`,
/// taking each symbolic byte as `0x00` and `0xff`.
pub(crate) fn precompile_word_byte_range(
    cx: &mut SymCx,
    input: &SymBytes,
    input_len: usize,
    offset: usize,
) -> (u64, u64) {
    let (min, max) = symbolic_input_word_range(cx, input, input_len, offset, 32);
    (min.saturating_to(), max.saturating_to())
}

pub(crate) fn symbolic_fixed_len_precompile_output(
    cx: &mut SymCx,
    algorithm: &'static str,
    input: &SymBytes,
    input_len: usize,
    output_len: usize,
) -> SymReturnData {
    let input_len_word = SymExpr::constant(cx, U256::from(input_len));
    let input = input.materialize(cx);
    let mut bytes = Vec::with_capacity(output_len);
    for chunk in 0..output_len.div_ceil(32) {
        let mut chunk_input = Vec::with_capacity(input.len() + 1);
        chunk_input.push(SymExpr::constant(cx, U256::from(chunk)));
        chunk_input.extend(input.iter().cloned());
        bytes.extend(
            symbolic_hash_word_with_len(cx, algorithm, chunk_input, input_len_word.clone())
                .into_byte_exprs(cx),
        );
    }
    bytes.truncate(output_len);
    SymReturnData::from_byte_exprs(cx, bytes)
}
