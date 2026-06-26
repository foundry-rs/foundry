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

pub(crate) fn exp_expr_for_concrete_exponent(base: Expr, exponent: usize) -> Expr {
    let mut expr = Expr::constant(U256::from(1));
    for _ in 0..exponent {
        expr = Expr::op(ExprOp::Mul, expr, base.clone());
    }
    expr.eval_const().map(Expr::constant).unwrap_or(expr)
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
    let bit_index = byte_index.to::<usize>() * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask = sign_bit - U256::from(1);
    if value & sign_bit == U256::ZERO { value & mask } else { value | !mask }
}

pub(crate) fn signextend_word(byte_index: U256, value: SymWord) -> SymWord {
    if byte_index >= U256::from(32) {
        return value;
    }
    if let Some(value) = value.as_const() {
        return SymWord::constant(signextend(byte_index, value));
    }
    let bit_index = byte_index.to::<usize>() * 8 + 7;
    let sign_bit = U256::from(1) << bit_index;
    let mask = sign_bit - U256::from(1);
    let value = value.into_expr();
    SymWord::expr(Expr::ite(
        BoolExpr::eq(
            Expr::op(ExprOp::And, value.clone(), Expr::constant(sign_bit)),
            Expr::constant(U256::ZERO),
        ),
        Expr::op(ExprOp::And, value.clone(), Expr::constant(mask)),
        Expr::op(ExprOp::Or, value, Expr::constant(!mask)),
    ))
}

pub(crate) fn signextend_word_dynamic(byte_index: SymWord, value: SymWord) -> SymWord {
    if let Some(byte_index) = byte_index.as_const() {
        return signextend_word(byte_index, value);
    }

    let byte_index = byte_index.into_expr();
    let mut result = value.clone_expr();
    for idx in (0..32).rev() {
        result = Expr::ite(
            BoolExpr::eq(byte_index.clone(), Expr::constant(U256::from(idx))),
            signextend_word(U256::from(idx), value.clone()).into_expr(),
            result,
        );
    }
    SymWord::expr(result)
}

pub(crate) fn byte_word(index: U256, word: SymWord) -> SymWord {
    if index >= U256::from(32) {
        return SymWord::zero();
    }
    let index = index.to::<usize>();
    if let Some(word) = word.as_const() {
        SymWord::constant(U256::from(word.to_be_bytes::<32>()[index]))
    } else {
        byte_expr(index, word.as_expr())
    }
}

pub(crate) fn byte_word_dynamic(index: SymWord, word: SymWord) -> SymWord {
    if let Some(index) = index.as_const() {
        return byte_word(index, word);
    }

    let index = index.into_expr();
    let mut result = Expr::constant(U256::ZERO);
    if let Some(word) = word.as_const() {
        let bytes = word.to_be_bytes::<32>();
        for idx in (0..32).rev() {
            result = Expr::ite(
                BoolExpr::eq(index.clone(), Expr::constant(U256::from(idx))),
                Expr::constant(U256::from(bytes[idx])),
                result,
            );
        }
    } else {
        let word = word.into_expr();
        for idx in (0..32).rev() {
            result = Expr::ite(
                BoolExpr::eq(index.clone(), Expr::constant(U256::from(idx))),
                byte_expr(idx, &word).into_expr(),
                result,
            );
        }
    }
    SymWord::expr(result)
}

/// Returns the byte extraction expression for a symbolic word.
pub(crate) fn byte_expr(index: usize, expr: &Expr) -> SymWord {
    debug_assert!(index < 32);
    if let Some(byte) = expr_known_byte(expr, index) {
        return SymWord::constant(U256::from(byte));
    }
    let shift = U256::from((31 - index) * 8);
    SymWord::expr(Expr::op(
        ExprOp::And,
        Expr::op(ExprOp::Shr, expr.clone(), Expr::constant(shift)),
        Expr::constant(U256::from(0xff)),
    ))
}

pub(crate) fn expr_known_byte(expr: &Expr, index: usize) -> Option<u8> {
    debug_assert!(index < 32);
    match expr.as_inner() {
        ExprInner::Const(value) => Some(value.to_be_bytes::<32>()[index]),
        ExprInner::Var(_) | ExprInner::GasLeft(_) | ExprInner::Keccak(_) | ExprInner::Hash(_) => {
            None
        }
        ExprInner::Not(value) => expr_known_byte(value, index).map(|byte| !byte),
        ExprInner::Ite(_, then_expr, else_expr) => {
            let then_byte = expr_known_byte(then_expr, index)?;
            let else_byte = expr_known_byte(else_expr, index)?;
            (then_byte == else_byte).then_some(then_byte)
        }
        ExprInner::Op(op, left, right) => match op {
            ExprOp::And => match (expr_known_byte(left, index), expr_known_byte(right, index)) {
                (Some(left), Some(right)) => Some(left & right),
                (Some(0), _) | (_, Some(0)) => Some(0),
                _ => None,
            },
            ExprOp::Or => Some(expr_known_byte(left, index)? | expr_known_byte(right, index)?),
            ExprOp::Xor => Some(expr_known_byte(left, index)? ^ expr_known_byte(right, index)?),
            ExprOp::Shl => {
                let shift = right.as_const()?;
                if shift >= U256::from(256) {
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
                let shift = right.as_const()?;
                if shift >= U256::from(256) {
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
        ExprInner::AddMod(_) | ExprInner::MulMod(_) => None,
    }
}

pub(crate) fn expr_known_word(expr: &Expr) -> Option<U256> {
    let mut bytes = [0u8; 32];
    for (idx, byte) in bytes.iter_mut().enumerate() {
        *byte = expr_known_byte(expr, idx)?;
    }
    Some(U256::from_be_bytes(bytes))
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

pub(crate) fn shift_left(value: SymWord, bits: usize) -> SymWord {
    if let Some(value) = value.as_const() {
        SymWord::constant(value << bits)
    } else {
        SymWord::expr(Expr::op(ExprOp::Shl, value.into_expr(), Expr::constant(U256::from(bits))))
    }
}

pub(crate) fn analyze_jumpdests(code: &SymCode) -> HashSet<usize> {
    let mut jumpdests = HashSet::default();
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

pub(crate) fn ensure_jumpdest(
    dest: usize,
    jumpdests: &HashSet<usize>,
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

pub(crate) fn abi_word_usize(word: &[u8]) -> Option<usize> {
    let value = abi_word(word)?;
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

pub(crate) const fn abi_word(word: &[u8]) -> Option<U256> {
    if word.len() != 32 {
        return None;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(word);
    Some(U256::from_be_bytes(bytes))
}
