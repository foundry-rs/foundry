use super::*;

pub(crate) fn mask_bits(value: U256, bits: usize) -> U256 {
    if bits >= 256 {
        value
    } else {
        let mask = (U256::from(1) << bits) - U256::from(1);
        value & mask
    }
}

pub(crate) fn address_word(address: Address) -> U256 {
    U256::from_be_bytes(address.into_word().0)
}

pub(crate) fn word_to_address(value: U256) -> Address {
    Address::from_word(value.to_be_bytes::<32>().into())
}

pub(crate) fn representative_symbolic_address(word: &SymWord) -> Address {
    let digest = keccak256(symbolic_address_key(word));
    let mut bytes = [0u8; 20];
    bytes[0] = 0xfe;
    bytes[1..].copy_from_slice(&digest[..19]);
    Address::from(bytes)
}

pub(crate) fn symbolic_address_key(word: &SymWord) -> String {
    if let Some(value) = word.as_const() {
        format!("concrete-address:{:?}", word_to_address(value))
    } else {
        let expr = word.as_expr();
        let bytes = address_byte_terms(expr)
            .map(|bytes| format!("{bytes:?}"))
            .unwrap_or_else(|| format!("{expr:?}"));
        format!("symbolic-address:{bytes}")
    }
}

pub(crate) fn address_match_condition(word: &SymWord, address: Address) -> BoolExpr {
    if let Some(word) = word.as_const() {
        return BoolExpr::constant(word == address_word(address));
    }
    let expr = word.as_expr();
    let Some(terms) = address_byte_terms(expr) else {
        return BoolExpr::eq(expr.clone(), SymExpr::constant(address_word(address)));
    };
    let bytes = address.as_slice();
    BoolExpr::and(
        terms
            .into_iter()
            .enumerate()
            .map(|(index, term)| BoolExpr::eq(term, SymExpr::constant(U256::from(bytes[index]))))
            .collect(),
    )
}

pub(crate) fn symbolic_address_equivalent(candidate: &SymWord, alias: &SymWord) -> bool {
    match (candidate.as_const(), alias.as_const()) {
        (Some(left), Some(right)) => word_to_address(left) == word_to_address(right),
        (None, None) => address_expr_equivalent(candidate.as_expr(), alias.as_expr()),
        _ => false,
    }
}

pub(crate) fn address_expr_equivalent(candidate: &SymExpr, alias: &SymExpr) -> bool {
    if candidate == alias {
        return true;
    }

    if let (Some(candidate), Some(alias)) =
        (address_byte_terms(candidate), address_byte_terms(alias))
    {
        return candidate == alias;
    }

    match candidate.as_inner() {
        ExprInner::Op(ExprOp::And, left, right) => {
            (is_address_mask(right) && address_expr_equivalent(left, alias))
                || (is_address_mask(left) && address_expr_equivalent(right, alias))
        }
        ExprInner::Op(ExprOp::Shr, value, shift) if is_shift_96(shift) => match value.as_inner() {
            ExprInner::Op(ExprOp::Shl, inner, inner_shift) if is_shift_96(inner_shift) => {
                address_expr_equivalent(inner, alias)
            }
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn address_byte_terms(expr: &SymExpr) -> Option<Vec<SymExpr>> {
    (12..32).map(|index| expr_byte_term(expr, index)).collect()
}

pub(crate) fn expr_byte_term(expr: &SymExpr, index: usize) -> Option<SymExpr> {
    debug_assert!(index < 32);

    match expr.as_inner() {
        ExprInner::Const(value) => {
            Some(SymExpr::constant(U256::from(value.to_be_bytes::<32>()[index])))
        }
        ExprInner::Var(_)
        | ExprInner::GasLeft(_)
        | ExprInner::Keccak { .. }
        | ExprInner::Hash { .. } => Some(extracted_byte_expr(expr, index)),
        ExprInner::Not(value) => Some(SymExpr::not(expr_byte_term(value, index)?)),
        ExprInner::Ite(cond, then_expr, else_expr) => Some(SymExpr::ite(
            cond.clone(),
            expr_byte_term(then_expr, index)?,
            expr_byte_term(else_expr, index)?,
        )),
        ExprInner::Op(op, left, right) => match op {
            ExprOp::And => expr_binary_byte_term(
                left,
                right,
                index,
                ExprOp::And,
                |byte| byte == 0xff,
                |byte| byte == 0,
            ),
            ExprOp::Or => {
                expr_binary_byte_term(left, right, index, ExprOp::Or, |byte| byte == 0, |_| false)
            }
            ExprOp::Xor => {
                expr_binary_byte_term(left, right, index, ExprOp::Xor, |byte| byte == 0, |_| false)
            }
            ExprOp::Shl => {
                let shift = right.eval_const()?;
                if shift >= U256::from(256) {
                    return Some(SymExpr::constant(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let source_index = index + shift / 8;
                if source_index >= 32 {
                    Some(SymExpr::constant(U256::ZERO))
                } else {
                    expr_byte_term(left, source_index)
                }
            }
            ExprOp::Shr => {
                let shift = right.eval_const()?;
                if shift >= U256::from(256) {
                    return Some(SymExpr::constant(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let byte_shift = shift / 8;
                if index < byte_shift {
                    Some(SymExpr::constant(U256::ZERO))
                } else {
                    expr_byte_term(left, index - byte_shift)
                }
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
        ExprInner::AddMod { .. } | ExprInner::MulMod { .. } => None,
    }
}

pub(crate) fn expr_binary_byte_term(
    left: &SymExpr,
    right: &SymExpr,
    index: usize,
    op: ExprOp,
    identity: impl Fn(u8) -> bool,
    absorbing: impl Fn(u8) -> bool,
) -> Option<SymExpr> {
    let left = expr_byte_term(left, index)?;
    let right = expr_byte_term(right, index)?;
    match (expr_byte_const(&left), expr_byte_const(&right)) {
        (Some(left), _) if absorbing(left) => Some(SymExpr::constant(U256::from(left))),
        (_, Some(right)) if absorbing(right) => Some(SymExpr::constant(U256::from(right))),
        (Some(left), _) if identity(left) => Some(right),
        (_, Some(right)) if identity(right) => Some(left),
        _ => Some(SymExpr::op(op, left, right)),
    }
}

pub(crate) fn expr_byte_const(expr: &SymExpr) -> Option<u8> {
    expr.as_const().map(|value| value.to::<u8>())
}

pub(crate) fn extracted_byte_expr(expr: &SymExpr, index: usize) -> SymExpr {
    SymExpr::op(
        ExprOp::And,
        SymExpr::op(ExprOp::Shr, expr.clone(), SymExpr::constant(U256::from((31 - index) * 8))),
        SymExpr::constant(U256::from(0xff)),
    )
}

pub(crate) fn is_address_mask(expr: &SymExpr) -> bool {
    expr.as_const() == Some((U256::from(1) << 160) - U256::from(1))
}

pub(crate) fn is_shift_96(expr: &SymExpr) -> bool {
    expr.as_const() == Some(U256::from(96))
}

pub(crate) fn stable_symbol(prefix: &'static str, input: impl AsRef<[u8]>) -> String {
    let digest = keccak256(input.as_ref());
    let mut symbol = String::with_capacity(prefix.len() + 17);
    symbol.push_str(prefix);
    symbol.push('_');
    for byte in &digest[..8] {
        let _ = write!(symbol, "{byte:02x}");
    }
    symbol
}
