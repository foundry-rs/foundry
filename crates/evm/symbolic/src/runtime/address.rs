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

impl SymExpr {
    pub(crate) fn representative_symbolic_address(&self) -> Address {
        let digest = keccak256(self.symbolic_address_key());
        let mut bytes = [0u8; 20];
        bytes[0] = 0xfe;
        bytes[1..].copy_from_slice(&digest[..19]);
        Address::from(bytes)
    }

    pub(crate) fn symbolic_address_key(&self) -> String {
        if let Some(value) = self.as_const() {
            format!("concrete-address:{:?}", word_to_address(value))
        } else {
            let bytes = self
                .address_byte_terms()
                .map(|bytes| format!("{bytes:?}"))
                .unwrap_or_else(|| format!("{self:?}"));
            format!("symbolic-address:{bytes}")
        }
    }

    pub(crate) fn address_match_condition(&self, address: Address) -> SymBoolExpr {
        if let Some(word) = self.as_const() {
            return SymBoolExpr::constant(word == address_word(address));
        }
        let Some(terms) = self.address_byte_terms() else {
            return SymBoolExpr::eq(self.clone(), Self::constant(address_word(address)));
        };
        let bytes = address.as_slice();
        SymBoolExpr::and(
            terms
                .into_iter()
                .enumerate()
                .map(|(index, term)| {
                    SymBoolExpr::eq(term, Self::constant(U256::from(bytes[index])))
                })
                .collect(),
        )
    }

    pub(crate) fn symbolic_address_equivalent(&self, alias: &Self) -> bool {
        match (self.as_const(), alias.as_const()) {
            (Some(left), Some(right)) => word_to_address(left) == word_to_address(right),
            (None, None) => self.address_expr_equivalent(alias),
            _ => false,
        }
    }

    fn address_expr_equivalent(&self, alias: &Self) -> bool {
        if self == alias {
            return true;
        }

        if let (Some(candidate), Some(alias)) =
            (self.address_byte_terms(), alias.address_byte_terms())
        {
            return candidate == alias;
        }

        match self.kind() {
            SymExprKind::Op(SymExprOp::And, left, right) => {
                (right.is_address_mask() && left.address_expr_equivalent(alias))
                    || (left.is_address_mask() && right.address_expr_equivalent(alias))
            }
            SymExprKind::Op(SymExprOp::Shr, value, shift) if shift.is_shift_96() => {
                match value.kind() {
                    SymExprKind::Op(SymExprOp::Shl, inner, inner_shift)
                        if inner_shift.is_shift_96() =>
                    {
                        inner.address_expr_equivalent(alias)
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn address_byte_terms(&self) -> Option<Vec<Self>> {
        (12..32).map(|index| self.byte_term(index)).collect()
    }

    fn is_address_mask(&self) -> bool {
        self.as_const() == Some((U256::from(1) << 160) - U256::from(1))
    }

    fn is_shift_96(&self) -> bool {
        self.as_const() == Some(U256::from(96))
    }
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
