use super::*;

pub(crate) fn failed_slot() -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..6].copy_from_slice(b"failed");
    U256::from_be_bytes(bytes)
}

pub(crate) fn pow_mod(base: U256, exponent: U256) -> U256 {
    let mut result = U256::from(1);
    let mut base = base;
    let mut exponent = exponent;
    while !exponent.is_zero() {
        if exponent & U256::from(1) == U256::from(1) {
            result = result.wrapping_mul(base);
        }
        exponent >>= 1;
        base = base.wrapping_mul(base);
    }
    result
}

pub(crate) fn exp_expr_for_concrete_exponent(
    cx: &mut SymCx,
    base: SymExpr,
    exponent: usize,
) -> SymExpr {
    if exponent == 0 {
        return SymExpr::one(cx);
    }
    if let Some(base) = base.as_const() {
        return SymExpr::constant(cx, pow_mod(base, U256::from(exponent)));
    }

    let mut expr = base.clone();
    for _ in 1..exponent {
        expr = SymExpr::op(cx, SymExprOp::Mul, expr, base.clone());
    }
    expr
}

pub(crate) fn slt(left: U256, right: U256) -> bool {
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    match (left_negative, right_negative) {
        (true, false) => true,
        (false, true) => false,
        _ => left < right,
    }
}

pub(crate) fn signed_abs(value: U256) -> U256 {
    if (value >> 255) == U256::from(1) { (!value).wrapping_add(U256::from(1)) } else { value }
}

pub(crate) fn sdiv(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    let quotient = signed_abs(left) / signed_abs(right);
    if left_negative ^ right_negative { (!quotient).wrapping_add(U256::from(1)) } else { quotient }
}

pub(crate) fn smod(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let remainder = signed_abs(left) % signed_abs(right);
    if left_negative { (!remainder).wrapping_add(U256::from(1)) } else { remainder }
}

pub(crate) fn signextend(byte_index: U256, value: U256) -> U256 {
    if byte_index >= U256::from(32) {
        return value;
    }
    let bit_index = usize::try_from(byte_index).expect("checked byte index") * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask = sign_bit - U256::from(1);
    if value & sign_bit == U256::ZERO { value & mask } else { value | !mask }
}

pub(crate) fn signextend_word(cx: &mut SymCx, byte_index: U256, value: SymExpr) -> SymExpr {
    if byte_index >= U256::from(32) {
        return value;
    }
    if let Some(value) = value.as_const() {
        return SymExpr::constant(cx, signextend(byte_index, value));
    }
    let bit_index = usize::try_from(byte_index).expect("checked byte index") * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask_value = sign_bit - U256::from(1);
    let sign_bit = SymExpr::constant(cx, sign_bit);
    let masked_sign = SymExpr::op(cx, SymExprOp::And, value.clone(), sign_bit);
    let zero = SymExpr::zero(cx);
    let condition = SymBoolExpr::eq(cx, masked_sign, zero);
    let inverse_mask = SymExpr::constant(cx, !mask_value);
    let mask = SymExpr::constant(cx, mask_value);
    let masked = SymExpr::op(cx, SymExprOp::And, value.clone(), mask);
    let extended = SymExpr::op(cx, SymExprOp::Or, value, inverse_mask);
    SymExpr::ite(cx, condition, masked, extended)
}

pub(crate) fn signextend_word_dynamic(
    cx: &mut SymCx,
    byte_index: SymExpr,
    value: SymExpr,
) -> SymExpr {
    if let Some(byte_index) = byte_index.as_const() {
        return signextend_word(cx, byte_index, value);
    }

    let mut result = value.clone();
    for idx in (0..31).rev() {
        let idx_expr = SymExpr::constant(cx, U256::from(idx));
        let condition = SymBoolExpr::eq(cx, byte_index.clone(), idx_expr);
        let value = signextend_word(cx, U256::from(idx), value.clone());
        result = SymExpr::ite(cx, condition, value, result);
    }
    result
}

pub(crate) fn byte_word(cx: &mut SymCx, index: U256, word: SymExpr) -> SymExpr {
    if index >= U256::from(32) {
        return SymExpr::zero(cx);
    }
    let index = usize::try_from(index).expect("checked byte index");
    if let Some(word) = word.as_const() {
        SymExpr::constant(cx, U256::from(word.to_be_bytes::<32>()[index]))
    } else {
        byte_expr(cx, index, &word)
    }
}

pub(crate) fn byte_word_dynamic(cx: &mut SymCx, index: SymExpr, word: SymExpr) -> SymExpr {
    if let Some(index) = index.as_const() {
        return byte_word(cx, index, word);
    }

    let mut result = SymExpr::zero(cx);
    if let Some(word) = word.as_const() {
        let bytes = word.to_be_bytes::<32>();
        for idx in (0..32).rev() {
            let idx_expr = SymExpr::constant(cx, U256::from(idx));
            let condition = SymBoolExpr::eq(cx, index.clone(), idx_expr);
            let byte = SymExpr::constant(cx, U256::from(bytes[idx]));
            result = SymExpr::ite(cx, condition, byte, result);
        }
    } else {
        for idx in (0..32).rev() {
            let idx_expr = SymExpr::constant(cx, U256::from(idx));
            let condition = SymBoolExpr::eq(cx, index.clone(), idx_expr);
            let byte = byte_expr(cx, idx, &word);
            result = SymExpr::ite(cx, condition, byte, result);
        }
    }
    result
}

/// Returns the byte extraction expression for a symbolic word.
pub(crate) fn byte_expr(cx: &mut SymCx, index: usize, expr: &SymExpr) -> SymExpr {
    debug_assert!(index < 32);
    if let Some(byte) = expr.known_byte(index) {
        return SymExpr::constant(cx, U256::from(byte));
    }
    expr.extracted_byte(cx, index)
}

pub(crate) fn sar(value: U256, shift: usize) -> U256 {
    if shift >= 256 {
        if (value >> 255) == U256::from(1) { U256::MAX } else { U256::ZERO }
    } else if shift == 0 {
        value
    } else if (value >> 255) == U256::from(1) {
        (value >> shift) | (U256::MAX << (256 - shift))
    } else {
        value >> shift
    }
}

pub(crate) fn shift_left(cx: &mut SymCx, value: SymExpr, bits: usize) -> SymExpr {
    if let Some(value) = value.as_const() {
        SymExpr::constant(cx, value << bits)
    } else {
        let bits = SymExpr::constant(cx, U256::from(bits));
        SymExpr::op(cx, SymExprOp::Shl, value, bits)
    }
}

pub(crate) fn ensure_jumpdest(dest: usize, jumpdests: &JumpTable) -> Result<(), SymbolicError> {
    if jumpdests.is_valid(dest) { Ok(()) } else { Err(SymbolicError::InvalidJump(dest)) }
}

pub(crate) fn is_assertion_revert(data: &[u8]) -> bool {
    is_assert_panic(data) || is_revert_assertion_failure(data)
}

pub(crate) fn is_assert_panic(data: &[u8]) -> bool {
    data.len() >= ABI_SELECTOR_PLUS_WORD_LEN
        && data.starts_with(&PANIC_SELECTOR)
        && abi_word(&data[4..ABI_SELECTOR_PLUS_WORD_LEN])
            .is_some_and(|code| code == ASSERT_PANIC_CODE)
}

pub(crate) fn is_revert_assertion_failure(data: &[u8]) -> bool {
    if data.len() < ERROR_DATA_MIN_LEN || !data.starts_with(&ERROR_SELECTOR) {
        return false;
    }

    let Some(offset) = abi_word_usize(&data[4..ABI_SELECTOR_PLUS_WORD_LEN]) else {
        return false;
    };
    let Some(length_offset) = 4usize.checked_add(offset) else {
        return false;
    };
    let Some(length_end) = length_offset.checked_add(32) else {
        return false;
    };
    if length_end > data.len() {
        return false;
    }

    let Some(length) = abi_word_usize(&data[length_offset..length_end]) else {
        return false;
    };
    let Some(message_end) = length_end.checked_add(length) else {
        return false;
    };
    if message_end > data.len() {
        return false;
    }

    std::str::from_utf8(&data[length_end..message_end])
        .is_ok_and(|message| message.contains(ASSERTION_FAILED_PREFIX))
}

pub(crate) fn abi_word_usize(word: &[u8]) -> Option<usize> {
    usize::try_from(abi_word(word)?).ok()
}

pub(crate) const fn abi_word(word: &[u8]) -> Option<U256> {
    if word.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(word);
    Some(U256::from_be_bytes(bytes))
}
