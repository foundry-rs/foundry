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
            let expr = self.symbolic_address_canonical();
            let bytes = expr
                .address_byte_terms_for_equivalence()
                .map(|bytes| format!("{bytes:?}"))
                .unwrap_or_else(|| format!("{expr:?}"));
            format!("symbolic-address:{bytes}")
        }
    }

    pub(crate) fn address_match_condition(&self, cx: &mut SymCx, address: Address) -> SymBoolExpr {
        if let Some(word) = self.as_const() {
            return SymBoolExpr::constant(cx, word == address_word(address));
        }
        let Some(terms) = self.address_byte_terms(cx) else {
            let address = Self::constant(cx, address_word(address));
            return SymBoolExpr::eq(cx, self.clone(), address);
        };
        let bytes = address.as_slice();
        let conditions = terms
            .into_iter()
            .enumerate()
            .map(|(index, term)| {
                let byte = Self::constant(cx, U256::from(bytes[index]));
                SymBoolExpr::eq(cx, term, byte)
            })
            .collect();
        SymBoolExpr::and(cx, conditions)
    }

    pub(crate) fn symbolic_address_equivalent(&self, alias: &Self) -> bool {
        match (self.as_const(), alias.as_const()) {
            (Some(left), Some(right)) => word_to_address(left) == word_to_address(right),
            (None, None) => self.address_expr_equivalent(alias),
            _ => false,
        }
    }

    fn address_expr_equivalent(&self, alias: &Self) -> bool {
        let this = self.symbolic_address_canonical();
        let alias = alias.symbolic_address_canonical();

        if this == alias {
            return true;
        }

        if let (Some(candidate), Some(alias)) =
            (this.address_byte_terms_for_equivalence(), alias.address_byte_terms_for_equivalence())
        {
            return candidate == alias;
        }

        match this.kind() {
            SymExprKind::BinOp(SymExprBinOp::And, left, right) => {
                (right.is_address_mask() && left.address_expr_equivalent(alias))
                    || (left.is_address_mask() && right.address_expr_equivalent(alias))
            }
            SymExprKind::BinOp(SymExprBinOp::Shr, value, shift) if shift.is_shift_96() => {
                match value.kind() {
                    SymExprKind::BinOp(SymExprBinOp::Shl, inner, inner_shift)
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

    fn symbolic_address_canonical(&self) -> &Self {
        match self.kind() {
            SymExprKind::BinOp(SymExprBinOp::And, left, right) if right.is_address_mask() => {
                left.symbolic_address_canonical()
            }
            SymExprKind::BinOp(SymExprBinOp::And, left, right) if left.is_address_mask() => {
                right.symbolic_address_canonical()
            }
            SymExprKind::BinOp(SymExprBinOp::Shr, value, shift) if shift.is_shift_96() => {
                match value.kind() {
                    SymExprKind::BinOp(SymExprBinOp::Shl, inner, inner_shift)
                        if inner_shift.is_shift_96() =>
                    {
                        inner.symbolic_address_canonical()
                    }
                    _ => self,
                }
            }
            _ => self,
        }
    }

    fn address_byte_terms(&self, cx: &mut SymCx) -> Option<Vec<Self>> {
        (12..32).map(|index| self.byte_term(cx, index)).collect()
    }

    fn address_byte_terms_for_equivalence(&self) -> Option<Vec<Self>> {
        (12..32).map(|index| self.extracted_byte_source(index)).collect()
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
