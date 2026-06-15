use super::*;

/// Returns the `mask_bits` address normalization helper result.
pub(crate) fn mask_bits(value: U256, bits: usize) -> U256 {
    if bits >= 256 {
        value
    } else {
        let mask = (U256::from(1) << bits) - U256::from(1);
        value & mask
    }
}

/// Returns the `address_word` address normalization helper result.
pub(crate) fn address_word(address: Address) -> U256 {
    U256::from_be_bytes(address.into_word().0)
}

/// Returns the `word_to_address` address normalization helper result.
pub(crate) fn word_to_address(value: U256) -> Address {
    Address::from_word(value.to_be_bytes::<32>().into())
}

/// Implements the `representative_symbolic_address` address normalization helper.
pub(crate) fn representative_symbolic_address(word: &SymWord) -> Address {
    let digest = keccak256(symbolic_address_key(word));
    let mut bytes = [0u8; 20];
    bytes[0] = 0xfe;
    bytes[1..].copy_from_slice(&digest[..19]);
    Address::from(bytes)
}

/// Returns the `symbolic_address_key` address normalization helper result.
pub(crate) fn symbolic_address_key(word: &SymWord) -> String {
    match word {
        SymWord::Concrete(value) => format!("concrete-address:{:?}", word_to_address(*value)),
        SymWord::Expr(expr) => {
            let bytes = address_byte_terms(expr)
                .map(|bytes| format!("{bytes:?}"))
                .unwrap_or_else(|| format!("{expr:?}"));
            format!("symbolic-address:{bytes}")
        }
    }
}

/// Returns the `address_match_condition` address normalization helper result.
pub(crate) fn address_match_condition(word: &SymWord, address: Address) -> BoolExpr {
    let expr = word.clone().into_expr();
    let Some(terms) = address_byte_terms(&expr) else {
        return BoolExpr::eq(expr, Expr::Const(address_word(address)));
    };
    let bytes = address.as_slice();
    BoolExpr::and(
        terms
            .into_iter()
            .enumerate()
            .map(|(index, term)| BoolExpr::eq(term, Expr::Const(U256::from(bytes[index]))))
            .collect(),
    )
}

/// Returns the `symbolic_address_equivalent` address normalization helper result.
pub(crate) fn symbolic_address_equivalent(candidate: &SymWord, alias: &SymWord) -> bool {
    match (candidate, alias) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            word_to_address(*left) == word_to_address(*right)
        }
        (SymWord::Expr(candidate), SymWord::Expr(alias)) => {
            address_expr_equivalent(candidate, alias)
        }
        _ => false,
    }
}

/// Returns the `address_expr_equivalent` address normalization helper result.
pub(crate) fn address_expr_equivalent(candidate: &Expr, alias: &Expr) -> bool {
    if candidate == alias {
        return true;
    }

    if let (Some(candidate), Some(alias)) =
        (address_byte_terms(candidate), address_byte_terms(alias))
    {
        return candidate == alias;
    }

    match candidate {
        Expr::Op(ExprOp::And, left, right) => {
            (is_address_mask(right) && address_expr_equivalent(left, alias))
                || (is_address_mask(left) && address_expr_equivalent(right, alias))
        }
        Expr::Op(ExprOp::Shr, value, shift) if is_shift_96(shift) => match value.as_ref() {
            Expr::Op(ExprOp::Shl, inner, inner_shift) if is_shift_96(inner_shift) => {
                address_expr_equivalent(inner, alias)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Returns the `address_byte_terms` address normalization helper result.
pub(crate) fn address_byte_terms(expr: &Expr) -> Option<Vec<Expr>> {
    (12..32).map(|index| expr_byte_term(expr, index)).collect()
}

/// Returns the `expr_byte_term` address normalization helper result.
pub(crate) fn expr_byte_term(expr: &Expr, index: usize) -> Option<Expr> {
    debug_assert!(index < 32);

    match expr {
        Expr::Const(value) => Some(Expr::Const(U256::from(value.to_be_bytes::<32>()[index]))),
        Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak { .. } | Expr::Hash { .. } => {
            Some(extracted_byte_expr(expr, index))
        }
        Expr::Not(value) => Some(Expr::Not(Box::new(expr_byte_term(value, index)?))),
        Expr::Ite(cond, then_expr, else_expr) => Some(Expr::Ite(
            cond.clone(),
            Box::new(expr_byte_term(then_expr, index)?),
            Box::new(expr_byte_term(else_expr, index)?),
        )),
        Expr::Op(op, left, right) => match op {
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
                let shift = expr_const_value(right)?;
                if shift >= U256::from(256) {
                    return Some(Expr::Const(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let source_index = index + shift / 8;
                if source_index >= 32 {
                    Some(Expr::Const(U256::ZERO))
                } else {
                    expr_byte_term(left, source_index)
                }
            }
            ExprOp::Shr => {
                let shift = expr_const_value(right)?;
                if shift >= U256::from(256) {
                    return Some(Expr::Const(U256::ZERO));
                }
                let shift = shift.to::<usize>();
                if shift % 8 != 0 {
                    return None;
                }
                let byte_shift = shift / 8;
                if index < byte_shift {
                    Some(Expr::Const(U256::ZERO))
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
        Expr::AddMod { .. } | Expr::MulMod { .. } => None,
    }
}

/// Returns the `expr_binary_byte_term` address normalization helper result.
pub(crate) fn expr_binary_byte_term(
    left: &Expr,
    right: &Expr,
    index: usize,
    op: ExprOp,
    identity: impl Fn(u8) -> bool,
    absorbing: impl Fn(u8) -> bool,
) -> Option<Expr> {
    let left = expr_byte_term(left, index)?;
    let right = expr_byte_term(right, index)?;
    match (expr_byte_const(&left), expr_byte_const(&right)) {
        (Some(left), _) if absorbing(left) => Some(Expr::Const(U256::from(left))),
        (_, Some(right)) if absorbing(right) => Some(Expr::Const(U256::from(right))),
        (Some(left), _) if identity(left) => Some(right),
        (_, Some(right)) if identity(right) => Some(left),
        _ => Some(Expr::op(op, left, right)),
    }
}

/// Returns the `expr_byte_const` address normalization helper result.
pub(crate) fn expr_byte_const(expr: &Expr) -> Option<u8> {
    let Expr::Const(value) = expr else { return None };
    Some(value.to::<u8>())
}

/// Implements the `extracted_byte_expr` address normalization helper.
pub(crate) fn extracted_byte_expr(expr: &Expr, index: usize) -> Expr {
    Expr::op(
        ExprOp::And,
        Expr::op(ExprOp::Shr, expr.clone(), Expr::Const(U256::from((31 - index) * 8))),
        Expr::Const(U256::from(0xff)),
    )
}

/// Returns whether `is_address_mask` holds.
pub(crate) fn is_address_mask(expr: &Expr) -> bool {
    matches!(expr, Expr::Const(value) if *value == ((U256::from(1) << 160) - U256::from(1)))
}

/// Returns whether `is_shift_96` holds.
pub(crate) fn is_shift_96(expr: &Expr) -> bool {
    matches!(expr, Expr::Const(value) if *value == U256::from(96))
}

/// Implements the `stable_symbol` address normalization helper.
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
