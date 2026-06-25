use super::*;

/// Returns the `failed_slot` EVM semantics helper result.
pub(crate) fn failed_slot() -> U256 {
    let mut bytes = [0u8; 32];
    bytes[..6].copy_from_slice(b"failed");
    U256::from_be_bytes(bytes)
}

/// Computes the `pow_mod` EVM semantics helper result.
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

/// Computes the `exp_expr_for_concrete_exponent` EVM semantics helper result.
pub(crate) fn exp_expr_for_concrete_exponent(base: Expr, exponent: usize) -> Expr {
    let mut expr = Expr::Const(U256::from(1));
    for _ in 0..exponent {
        expr = Expr::op(ExprOp::Mul, expr, base.clone());
    }
    expr_const_value(&expr).map(Expr::Const).unwrap_or(expr)
}

/// Implements the `slt` EVM semantics helper.
pub(crate) fn slt(left: U256, right: U256) -> bool {
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    match (left_negative, right_negative) {
        (true, false) => true,
        (false, true) => false,
        _ => left < right,
    }
}

/// Implements the `signed_abs` EVM semantics helper.
pub(crate) fn signed_abs(value: U256) -> U256 {
    if (value >> 255) == U256::from(1) { (!value).wrapping_add(U256::from(1)) } else { value }
}

/// Implements the `sdiv` EVM semantics helper.
pub(crate) fn sdiv(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let right_negative = (right >> 255) == U256::from(1);
    let quotient = signed_abs(left) / signed_abs(right);
    if left_negative ^ right_negative { (!quotient).wrapping_add(U256::from(1)) } else { quotient }
}

/// Implements the `smod` EVM semantics helper.
pub(crate) fn smod(left: U256, right: U256) -> U256 {
    if right.is_zero() {
        return U256::ZERO;
    }
    let left_negative = (left >> 255) == U256::from(1);
    let remainder = signed_abs(left) % signed_abs(right);
    if left_negative { (!remainder).wrapping_add(U256::from(1)) } else { remainder }
}

/// Implements the `signextend` EVM semantics helper.
pub(crate) fn signextend(byte_index: U256, value: U256) -> U256 {
    if byte_index >= U256::from(32) {
        return value;
    }
    let bit_index = byte_index.to::<usize>() * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask = sign_bit - U256::from(1);
    if value & sign_bit == U256::ZERO { value & mask } else { value | !mask }
}

/// Implements the `signextend_word` EVM semantics helper.
pub(crate) fn signextend_word(byte_index: U256, value: SymWord) -> SymWord {
    if byte_index >= U256::from(32) {
        return value;
    }
    match value {
        SymWord::Concrete(value) => SymWord::Concrete(signextend(byte_index, value)),
        value => {
            let bit_index = byte_index.to::<usize>() * 8 + 7;
            let sign_bit = U256::from(1) << bit_index;
            let mask = sign_bit - U256::from(1);
            let value = value.into_expr();
            SymWord::Expr(Expr::Ite(
                Box::new(BoolExpr::eq(
                    Expr::op(ExprOp::And, value.clone(), Expr::Const(sign_bit)),
                    Expr::Const(U256::ZERO),
                )),
                Box::new(Expr::op(ExprOp::And, value.clone(), Expr::Const(mask))),
                Box::new(Expr::op(ExprOp::Or, value, Expr::Const(!mask))),
            ))
        }
    }
}

/// Implements the `signextend_word_dynamic` EVM semantics helper.
pub(crate) fn signextend_word_dynamic(byte_index: SymWord, value: SymWord) -> SymWord {
    if let SymWord::Concrete(byte_index) = byte_index {
        return signextend_word(byte_index, value);
    }

    let byte_index = byte_index.into_expr();
    let mut result = value.clone().into_expr();
    for idx in (0..32).rev() {
        result = Expr::Ite(
            Box::new(BoolExpr::eq(byte_index.clone(), Expr::Const(U256::from(idx)))),
            Box::new(signextend_word(U256::from(idx), value.clone()).into_expr()),
            Box::new(result),
        );
    }
    SymWord::Expr(result)
}

/// Returns the `byte_word` EVM semantics helper result.
pub(crate) fn byte_word(index: U256, word: SymWord) -> SymWord {
    if index >= U256::from(32) {
        return SymWord::zero();
    }
    let index = index.to::<usize>();
    match word {
        SymWord::Concrete(word) => SymWord::Concrete(U256::from(word.to_be_bytes::<32>()[index])),
        word => {
            let expr = word.into_expr();
            if let Some(byte) = expr_known_byte(&expr, index) {
                return SymWord::Concrete(U256::from(byte));
            }
            let shift = U256::from((31 - index) * 8);
            SymWord::from_expr(Expr::op(
                ExprOp::And,
                Expr::op(ExprOp::Shr, expr, Expr::Const(shift)),
                Expr::Const(U256::from(0xff)),
            ))
        }
    }
}

/// Returns the `byte_word_dynamic` EVM semantics helper result.
pub(crate) fn byte_word_dynamic(index: SymWord, word: SymWord) -> SymWord {
    if let SymWord::Concrete(index) = index {
        return byte_word(index, word);
    }

    let index = index.into_expr();
    let mut result = Expr::Const(U256::ZERO);
    for idx in (0..32).rev() {
        result = Expr::Ite(
            Box::new(BoolExpr::eq(index.clone(), Expr::Const(U256::from(idx)))),
            Box::new(byte_word(U256::from(idx), word.clone()).into_expr()),
            Box::new(result),
        );
    }
    SymWord::from_expr(result)
}

/// Returns the `expr_known_byte` EVM semantics helper result.
pub(crate) fn expr_known_byte(expr: &Expr, index: usize) -> Option<u8> {
    debug_assert!(index < 32);
    match expr {
        Expr::Const(value) => Some(value.to_be_bytes::<32>()[index]),
        Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => None,
        Expr::Not(value) => expr_known_byte(value, index).map(|byte| !byte),
        Expr::Ite(_, then_expr, else_expr) => {
            let then_byte = expr_known_byte(then_expr, index)?;
            let else_byte = expr_known_byte(else_expr, index)?;
            (then_byte == else_byte).then_some(then_byte)
        }
        Expr::Op(op, left, right) => match op {
            ExprOp::And => match (expr_known_byte(left, index), expr_known_byte(right, index)) {
                (Some(left), Some(right)) => Some(left & right),
                (Some(0), _) | (_, Some(0)) => Some(0),
                _ => None,
            },
            ExprOp::Or => Some(expr_known_byte(left, index)? | expr_known_byte(right, index)?),
            ExprOp::Xor => Some(expr_known_byte(left, index)? ^ expr_known_byte(right, index)?),
            ExprOp::Shl => {
                let Expr::Const(shift) = right.as_ref() else { return None };
                if *shift >= U256::from(256) {
                    return Some(0);
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let source_index = index + shift / 8;
                if source_index >= 32 { Some(0) } else { expr_known_byte(left, source_index) }
            }
            ExprOp::Shr => {
                let Expr::Const(shift) = right.as_ref() else { return None };
                if *shift >= U256::from(256) {
                    return Some(0);
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let byte_shift = shift / 8;
                if index < byte_shift { Some(0) } else { expr_known_byte(left, index - byte_shift) }
            }
            ExprOp::Add
            | ExprOp::Sub
            | ExprOp::Mul
            | ExprOp::UDiv
            | ExprOp::URem
            | ExprOp::SDiv
            | ExprOp::SRem
            | ExprOp::Sar => None,
        },
        Expr::AddMod { .. } | Expr::MulMod { .. } => None,
    }
}

/// Returns the `expr_known_word` EVM semantics helper result.
pub(crate) fn expr_known_word(expr: &Expr) -> Option<U256> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = expr_known_byte(expr, idx)?;
    }
    Some(U256::from_be_bytes(bytes))
}

/// Implements the `sar` EVM semantics helper.
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

/// Computes the `shift_left` EVM semantics helper result.
pub(crate) fn shift_left(value: SymWord, bits: usize) -> SymWord {
    match value {
        SymWord::Concrete(value) => SymWord::Concrete(value << bits),
        value => {
            SymWord::Expr(Expr::op(ExprOp::Shl, value.into_expr(), Expr::Const(U256::from(bits))))
        }
    }
}

/// Computes the `analyze_jumpdests` EVM semantics helper result.
pub(crate) fn analyze_jumpdests(code: &SymCode) -> BTreeSet<usize> {
    let mut jumpdests = BTreeSet::new();
    let mut pc = 0;
    while pc < code.len() {
        let op = code.analysis_opcode(pc).unwrap_or(opcode::STOP);
        if op == opcode::JUMPDEST {
            jumpdests.insert(pc);
            pc += 1;
        } else if (opcode::PUSH1..=opcode::PUSH32).contains(&op) {
            pc += 1 + (op - opcode::PUSH1 + 1) as usize;
        } else {
            pc += 1;
        }
    }
    jumpdests
}

/// Computes the `ensure_jumpdest` EVM semantics helper result.
pub(crate) fn ensure_jumpdest(
    dest: usize,
    jumpdests: &BTreeSet<usize>,
) -> Result<(), SymbolicError> {
    if jumpdests.contains(&dest) { Ok(()) } else { Err(SymbolicError::InvalidJump(dest)) }
}

/// Returns whether `is_assertion_revert` holds.
pub(crate) fn is_assertion_revert(data: &[u8]) -> bool {
    is_assert_panic(data) || is_revert_assertion_failure(data)
}

/// Returns whether `is_assert_panic` holds.
pub(crate) fn is_assert_panic(data: &[u8]) -> bool {
    data.len() >= ABI_SELECTOR_PLUS_WORD_LEN
        && data.starts_with(&PANIC_SELECTOR)
        && abi_word(&data[4..ABI_SELECTOR_PLUS_WORD_LEN])
            .is_some_and(|code| code == ASSERT_PANIC_CODE)
}

/// Returns whether `is_revert_assertion_failure` holds.
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

/// Returns the `abi_word_usize` EVM semantics helper result.
pub(crate) fn abi_word_usize(word: &[u8]) -> Option<usize> {
    let value = abi_word(word)?;
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

/// Returns the `abi_word` EVM semantics helper result.
pub(crate) const fn abi_word(word: &[u8]) -> Option<U256> {
    if word.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(word);
    Some(U256::from_be_bytes(bytes))
}
