use super::*;

pub(crate) fn is_known_cheatcode(address: Address) -> bool {
    address == CHEATCODE_ADDRESS || address == SYMBOLIC_VM_COMPAT_ADDRESS
}

pub(crate) fn is_console(address: Address) -> bool {
    address == HARDHAT_CONSOLE_ADDRESS
}

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

pub(crate) fn precompile_address(number: u8) -> Address {
    let mut bytes = [0u8; 20];
    bytes[PRECOMPILE_ADDRESS_LEADING_ZEROS] = number;
    Address::from(bytes)
}

pub(crate) fn is_supported_precompile(address: Address, spec_id: SpecId) -> bool {
    precompile_number_for_spec(address, spec_id).is_some()
}

pub(crate) fn execute_precompile(
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
        Ok(output) => Ok(Some(SymReturnData::from_concrete_bytes(output.bytes.to_vec()))),
        Err(_) => Ok(None),
    }
}

pub(crate) fn execute_symbolic_precompile(
    address: Address,
    input: Vec<SymExpr>,
    input_len: SymExpr,
    spec_id: SpecId,
) -> Result<Option<SymReturnData>, SymbolicError> {
    if input.iter().all(|byte| byte.as_const().is_some())
        && let Some(input_len) = input_len.as_const()
        && input_len <= U256::from(input.len())
    {
        let input_len = input_len.to::<usize>();
        let input = concrete_bytes(&input[..input_len], "symbolic precompile input")?;
        return execute_precompile(address, &input, spec_id);
    }

    match precompile_number_for_spec(address, spec_id) {
        Some(1) => {
            let word = symbolic_hash_word_with_len("ecrecover", input, input_len);
            let mut bytes = vec![SymExpr::zero(); 12];
            bytes.extend((12..32).map(|idx| byte_word(U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_symbolic_bytes(bytes)))
        }
        Some(2) => Ok(Some(SymReturnData::from_symbolic_bytes(word_bytes(
            symbolic_hash_word_with_len("sha256", input, input_len),
        )))),
        Some(3) => {
            let word = symbolic_hash_word_with_len("ripemd160", input, input_len);
            let mut bytes = vec![SymExpr::zero(); 12];
            bytes.extend((12..32).map(|idx| byte_word(U256::from(idx), word.clone())));
            Ok(Some(SymReturnData::from_symbolic_bytes(bytes)))
        }
        Some(4) => Ok(Some(SymReturnData::from_symbolic_bytes_with_len(input, input_len))),
        Some(5) => symbolic_modexp_precompile(input, input_len),
        Some(6) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if input_has_symbolic_bytes(&input, input_len) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_add", input, input_len, 64)))
        }
        Some(7) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if input_has_symbolic_bytes(&input, input_len) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_mul", input, input_len, 64)))
        }
        Some(8) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len % 192 != 0 {
                return Ok(None);
            }
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            if input_has_symbolic_bytes(&input, input_len) {
                return Err(SymbolicError::Unsupported(
                    "symbolic bn254 precompile validity not modeled",
                ));
            }
            Ok(Some(symbolic_fixed_len_precompile_output("bn254_pairing", input, input_len, 32)))
        }
        Some(9) => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            if input_len != 213 {
                return Ok(None);
            }
            if input_len > input.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
            }
            match input.get(212) {
                Some(flag)
                    if flag
                        .as_const()
                        .is_some_and(|flag| flag.is_zero() || flag == U256::from(1)) => {}
                Some(flag) if flag.as_const().is_some() => return Ok(None),
                Some(_) => {
                    return Err(SymbolicError::Unsupported(
                        "symbolic blake2f precompile final flag not modeled",
                    ));
                }
                None => {
                    return Err(SymbolicError::Unsupported(
                        "out-of-bounds symbolic precompile input",
                    ));
                }
            }
            Ok(Some(symbolic_fixed_len_precompile_output("blake2f", input, input_len, 64)))
        }
        Some(10) => Err(SymbolicError::Unsupported("KZG handled by execute_kzg_precompile_call")),
        _ => {
            let input_len = input_len.into_usize("symbolic precompile input")?;
            let input = concrete_bytes(
                input
                    .get(..input_len)
                    .ok_or(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"))?,
                "symbolic precompile input",
            )?;
            execute_precompile(address, &input, spec_id)
        }
    }
}

fn input_has_symbolic_bytes(input: &[SymExpr], input_len: usize) -> bool {
    input.iter().take(input_len).any(|byte| byte.as_const().is_none())
}

pub(crate) fn symbolic_modexp_precompile(
    input: Vec<SymExpr>,
    input_len: SymExpr,
) -> Result<Option<SymReturnData>, SymbolicError> {
    let input_len = input_len.into_usize("symbolic precompile input")?;
    if input_len > input.len() {
        return Err(SymbolicError::Unsupported("out-of-bounds symbolic precompile input"));
    }

    let modulus_len = concrete_precompile_word_at(&input, 64)?;
    let modulus_len = u256_to_usize(modulus_len)
        .ok_or(SymbolicError::Unsupported("symbolic modexp output length"))?;
    if modulus_len > 4096 {
        return Err(SymbolicError::Unsupported("symbolic modexp output length"));
    }
    Ok(Some(symbolic_fixed_len_precompile_output("modexp", input, input_len, modulus_len)))
}

pub(crate) fn concrete_precompile_word_at(
    input: &[SymExpr],
    offset: usize,
) -> Result<U256, SymbolicError> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = match input.get(offset + idx) {
            Some(word) => match word.as_const() {
                Some(byte) => byte.to::<u8>(),
                None => {
                    return Err(SymbolicError::Unsupported("symbolic precompile length header"));
                }
            },
            None => 0,
        };
    }
    Ok(U256::from_be_bytes(bytes))
}

pub(crate) fn symbolic_fixed_len_precompile_output(
    algorithm: &'static str,
    input: Vec<SymExpr>,
    input_len: usize,
    output_len: usize,
) -> SymReturnData {
    let input_len_word = SymExpr::constant(U256::from(input_len));
    let mut bytes = Vec::with_capacity(output_len);
    for chunk in 0..output_len.div_ceil(32) {
        let mut chunk_input = Vec::with_capacity(input.len() + 1);
        chunk_input.push(SymExpr::constant(U256::from(chunk)));
        chunk_input.extend(input.iter().cloned());
        bytes.extend(word_bytes(symbolic_hash_word_with_len(
            algorithm,
            chunk_input,
            input_len_word.clone(),
        )));
    }
    bytes.truncate(output_len);
    SymReturnData::from_symbolic_bytes(bytes)
}
