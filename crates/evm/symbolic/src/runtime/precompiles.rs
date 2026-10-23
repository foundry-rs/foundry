use super::*;

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

pub(crate) fn execute_precompile(
    cx: &mut SymCx,
    address: Address,
    input: &[u8],
    spec_id: SpecId,
) -> Result<Option<SymReturnData>, SymbolicError> {
    let output = match precompile_number_for_spec(address, spec_id) {
        Some(1) => secp256k1::ec_recover_run(input, u64::MAX),
        Some(2) => hash::sha256_run(input, u64::MAX),
        Some(3) => hash::ripemd160_run(input, u64::MAX),
        Some(4) => identity::identity_run(input, u64::MAX),
        Some(5) => modexp::berlin_run(input, u64::MAX),
        Some(6) => bn254::run_add(input, bn254::add::ISTANBUL_ADD_GAS_COST, u64::MAX),
        Some(7) => bn254::run_mul(input, bn254::mul::ISTANBUL_MUL_GAS_COST, u64::MAX),
        Some(8) => bn254::run_pair(
            input,
            bn254::pair::ISTANBUL_PAIR_PER_POINT,
            bn254::pair::ISTANBUL_PAIR_BASE,
            u64::MAX,
        ),
        Some(9) => blake2::run(input, u64::MAX),
        Some(10) => kzg_point_evaluation::run(input, u64::MAX),
        _ => return Err(SymbolicError::Unsupported("unsupported precompile")),
    };

    match output {
        Ok(output) => Ok(Some(SymReturnData::from_concrete_bytes(cx, output.bytes.to_vec()))),
        Err(_) => Ok(None),
    }
}

pub(crate) fn execute_symbolic_precompile(
    cx: &mut SymCx,
    address: Address,
    input: SymBytes,
    input_len: SymExpr,
    spec_id: SpecId,
) -> Result<Option<SymReturnData>, SymbolicError> {
    if let Some(input_len) = input_len.as_const()
        && let Ok(input_len) = usize::try_from(input_len)
        && input_len <= input.len()
        && let Ok(input) =
            input.slice_concrete(cx, 0, input_len).concrete_bytes(cx, "symbolic precompile input")
    {
        return execute_precompile(cx, address, &input, spec_id);
    }

    match precompile_number_for_spec(address, spec_id) {
        Some(1) => {
            // ECRECOVER ignores trailing bytes and pads short input with zeros.
            let input = SymBytes::sized(cx, input, input_len, 128);
            if let Ok(input) = input.concrete_bytes(cx, "symbolic ecrecover input") {
                return execute_precompile(cx, address, &input, spec_id);
            }
            let v = input.word_at(cx, 32);
            let v27 = SymBoolExpr::eq_word_const(cx, &v, U256::from(27));
            let v28 = SymBoolExpr::eq_word_const(cx, &v, U256::from(28));
            let valid_v = SymBoolExpr::or(cx, vec![v27, v28]);
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
            let input = input.materialize(cx);
            let word = symbolic_hash_word_with_len(cx, "sha256", input, input_len);
            let bytes = word.into_byte_exprs(cx);
            Ok(Some(SymReturnData::from_byte_exprs(cx, bytes)))
        }
        Some(3) => {
            let input = input.materialize(cx);
            let word = symbolic_hash_word_with_len(cx, "ripemd160", input, input_len);
            let mut bytes = vec![SymExpr::zero(cx); 12];
            bytes.extend((12..32).map(|idx| byte_word(cx, U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_byte_exprs(cx, bytes)))
        }
        Some(4) => Ok(Some(SymReturnData { len_word: input_len, bytes: input })),
        Some(5) => symbolic_modexp_precompile(cx, &input, input_len),
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
            execute_precompile(cx, address, &input, spec_id)
        }
    }
}

pub(crate) fn symbolic_modexp_precompile(
    cx: &mut SymCx,
    input: &SymBytes,
    input_len: SymExpr,
) -> Result<Option<SymReturnData>, SymbolicError> {
    let input_len = input_len.as_usize_or("symbolic precompile input")?;
    if input_len > input.len() {
        return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
    }

    let modulus_len = concrete_precompile_word_at(cx, input, 64)?;
    let modulus_len = usize::try_from(modulus_len)
        .ok()
        .ok_or(SymbolicError::Unsupported("symbolic modexp output length"))?;
    if modulus_len > 4096 {
        return Err(SymbolicError::Unsupported("symbolic modexp output length"));
    }
    Ok(Some(symbolic_fixed_len_precompile_output(cx, "modexp", input, input_len, modulus_len)))
}

pub(crate) fn concrete_precompile_word_at(
    cx: &mut SymCx,
    input: &SymBytes,
    offset: usize,
) -> Result<U256, SymbolicError> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        let word = input.byte(cx, offset + idx);
        *byte = word.as_const_or("symbolic precompile length header")?.to::<u8>();
    }
    Ok(U256::from_be_bytes(bytes))
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
