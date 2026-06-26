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

pub(crate) fn representative_symbolic_address(word: &SymExpr) -> Address {
    let digest = keccak256(symbolic_address_key(word));
    let mut bytes = [0u8; 20];
    bytes[0] = 0xfe;
    bytes[1..].copy_from_slice(&digest[..19]);
    Address::from(bytes)
}

pub(crate) fn symbolic_address_key(word: &SymExpr) -> String {
    if let Some(value) = word.as_const() {
        format!("concrete-address:{:?}", word_to_address(value))
    } else {
        let expr = word;
        let bytes = address_byte_terms(expr)
            .map(|bytes| format!("{bytes:?}"))
            .unwrap_or_else(|| format!("{expr:?}"));
        format!("symbolic-address:{bytes}")
    }
}

pub(crate) fn address_match_condition(word: &SymExpr, address: Address) -> SymBoolExpr {
    if let Some(word) = word.as_const() {
        return SymBoolExpr::constant(word == address_word(address));
    }
    let expr = word;
    let Some(terms) = address_byte_terms(expr) else {
        return SymBoolExpr::eq(expr.clone(), SymExpr::constant(address_word(address)));
    };
    let bytes = address.as_slice();
    SymBoolExpr::and(
        terms
            .into_iter()
            .enumerate()
            .map(|(index, term)| SymBoolExpr::eq(term, SymExpr::constant(U256::from(bytes[index]))))
            .collect(),
    )
}

pub(crate) fn symbolic_address_equivalent(candidate: &SymExpr, alias: &SymExpr) -> bool {
    match (candidate.as_const(), alias.as_const()) {
        (Some(left), Some(right)) => word_to_address(left) == word_to_address(right),
        (None, None) => address_expr_equivalent(candidate, alias),
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

    match candidate.kind() {
        SymExprKind::Op(SymExprOp::And, left, right) => {
            (is_address_mask(right) && address_expr_equivalent(left, alias))
                || (is_address_mask(left) && address_expr_equivalent(right, alias))
        }
        SymExprKind::Op(SymExprOp::Shr, value, shift) if is_shift_96(shift) => match value.kind() {
            SymExprKind::Op(SymExprOp::Shl, inner, inner_shift) if is_shift_96(inner_shift) => {
                address_expr_equivalent(inner, alias)
            }
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn address_byte_terms(expr: &SymExpr) -> Option<Vec<SymExpr>> {
    (12..32).map(|index| expr.byte_term(index)).collect()
}

pub(crate) fn is_address_mask(expr: &SymExpr) -> bool {
    expr.as_const() == Some((U256::from(1) << 160) - U256::from(1))
}

pub(crate) fn is_shift_96(expr: &SymExpr) -> bool {
    expr.as_const() == Some(U256::from(96))
}

pub(crate) fn stable_symbol(prefix: &'static str, input: &[u8]) -> Symbol {
    let digest = keccak256(input);
    let mut symbol = String::with_capacity(prefix.len() + 17);
    symbol.push_str(prefix);
    symbol.push('_');
    for byte in &digest[..8] {
        let _ = write!(symbol, "{byte:02x}");
    }
    Symbol::intern(&symbol)
}
