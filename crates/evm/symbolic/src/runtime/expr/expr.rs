use super::{hashcons::HashConsed, *};

// Boolean selector recovery is an optional expression rewrite. Bound it to one word's worth of
// unique nodes so adversarial expression trees cannot make construction unbounded.
const MAX_BITWISE_BOOL_WORD_VISITS: usize = 256;

impl SymExpr {
    pub(crate) fn select_storage_write(
        self,
        cx: &mut SymCx,
        write_key: Self,
        write_value: Self,
        base: Self,
    ) -> Self {
        if write_value == base {
            return base;
        }
        let condition = self.storage_key_eq(cx, &write_key);
        match condition.as_const() {
            Some(true) => write_value,
            Some(false) => base,
            None => Self::ite(cx, condition, write_value, base),
        }
    }

    pub(crate) fn storage_key_eq(&self, cx: &mut SymCx, write_key: &Self) -> SymBoolExpr {
        if let (Some(read_root), Some(write_root)) =
            (self.storage_mapping_root_slot(cx), write_key.storage_mapping_root_slot(cx))
            && read_root != write_root
        {
            return SymBoolExpr::constant(cx, false);
        }
        match (self.storage_layout_key(cx), write_key.storage_layout_key(cx)) {
            (Some((read_base, read_offset)), Some((write_base, write_offset))) => {
                let read_base = read_base
                    .storage_base_eq(cx, &write_base)
                    .unwrap_or_else(|| SymBoolExpr::eq(cx, read_base, write_base));
                let read_offset = SymBoolExpr::eq(cx, read_offset, write_offset);
                SymBoolExpr::and(cx, vec![read_base, read_offset])
            }
            (Some(_), None) if write_key.as_const().is_some() => SymBoolExpr::constant(cx, false),
            (None, Some(_)) if self.as_const().is_some() => SymBoolExpr::constant(cx, false),
            _ => SymBoolExpr::eq(cx, self.clone(), write_key.clone()),
        }
    }

    fn storage_base_eq(&self, cx: &mut SymCx, other: &Self) -> Option<SymBoolExpr> {
        let read = self.storage_mapping_key(cx)?;
        let write = other.storage_mapping_key(cx)?;

        let key_eq = storage_mapping_key_eq(cx, &read, &write);
        let slot_eq = read
            .slot
            .storage_base_eq(cx, &write.slot)
            .unwrap_or_else(|| SymBoolExpr::eq(cx, read.slot, write.slot));
        Some(SymBoolExpr::and(cx, vec![key_eq, slot_eq]))
    }

    pub(crate) fn storage_mapping_key(&self, cx: &mut SymCx) -> Option<StorageMappingKey> {
        let bytes = self.storage_mapping_key_bytes(cx)?;
        let key_bytes = &bytes[..32];
        let preserve_key_bytes =
            (!storage_mapping_key_bytes_form_compact_word(key_bytes)).then(|| key_bytes.to_vec());
        let key = Self::from_bytes(cx, key_bytes.iter().cloned());
        let slot = Self::from_bytes(cx, bytes[32..64].iter().cloned());
        Some(StorageMappingKey { key, key_bytes: preserve_key_bytes, slot })
    }

    pub(crate) fn storage_mapping_provenance_observed_with(
        &self,
        cx: &mut SymCx,
        mut observed_preimage: impl FnMut(&Self) -> Option<Arc<[Self]>>,
    ) -> Option<SymbolicMappingProvenance> {
        let mut current = self.clone();
        let mut keys = Vec::new();
        let mut visited = Vec::new();
        loop {
            if visited.contains(&current) {
                return None;
            }
            visited.push(current.clone());
            let bytes = observed_preimage(&current)?;
            if bytes.len() != 64 {
                return None;
            }
            let key = Self::from_bytes(cx, bytes[..32].iter().cloned());
            keys.push(key);
            current = Self::from_bytes(cx, bytes[32..64].iter().cloned());
            match current.kind() {
                SymExprKind::Const(root_slot) if observed_preimage(&current).is_none() => {
                    keys.reverse();
                    return Some(SymbolicMappingProvenance { root_slot: *root_slot, keys });
                }
                SymExprKind::Const(_) | SymExprKind::Keccak { .. } => {}
                _ => return None,
            }
        }
    }

    fn storage_mapping_root_slot(&self, cx: &mut SymCx) -> Option<U256> {
        let bytes = self.storage_mapping_key_bytes(cx)?;
        let slot = Self::from_bytes(cx, bytes[32..64].iter().cloned());
        match slot.kind() {
            SymExprKind::Const(value) if cx.concrete_keccak_preimage(*value).is_some() => {
                slot.storage_mapping_root_slot(cx)
            }
            SymExprKind::Const(slot) => Some(*slot),
            SymExprKind::Keccak { .. } => slot.storage_mapping_root_slot(cx),
            _ => None,
        }
    }

    fn storage_mapping_key_bytes(&self, cx: &SymCx) -> Option<Arc<[Self]>> {
        match self.kind() {
            SymExprKind::Keccak { len, bytes, .. }
                if len.as_const() == Some(U256::from(64)) && bytes.len() >= 64 =>
            {
                Some(bytes.clone())
            }
            SymExprKind::Const(hash) => cx.concrete_keccak_preimage(*hash),
            _ => None,
        }
    }

    fn storage_layout_key(&self, cx: &mut SymCx) -> Option<(Self, Self)> {
        match self.kind() {
            SymExprKind::Keccak { .. } => Some((self.clone(), Self::zero(cx))),
            SymExprKind::Const(hash) if cx.concrete_keccak_preimage(*hash).is_some() => {
                Some((self.clone(), Self::zero(cx)))
            }
            SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                if let Some((base, offset)) = left.storage_layout_key(cx)
                    && !right.contains_keccak()
                {
                    let offset = Self::binop(cx, SymBinOp::Add, offset, right.clone());
                    return Some((base, offset));
                }
                if let Some((base, offset)) = right.storage_layout_key(cx)
                    && !left.contains_keccak()
                {
                    let offset = Self::binop(cx, SymBinOp::Add, offset, left.clone());
                    return Some((base, offset));
                }
                None
            }
            _ => None,
        }
    }
}

pub(crate) struct StorageMappingKey {
    key: SymExpr,
    key_bytes: Option<Vec<SymExpr>>,
    slot: SymExpr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SymbolicMappingProvenance {
    pub(crate) root_slot: U256,
    pub(crate) keys: Vec<SymExpr>,
}

fn storage_mapping_key_eq(
    cx: &mut SymCx,
    read: &StorageMappingKey,
    write: &StorageMappingKey,
) -> SymBoolExpr {
    if read.key_bytes.is_some() || write.key_bytes.is_some() {
        let read_owned;
        let read_bytes = if let Some(bytes) = read.key_bytes.as_deref() {
            bytes
        } else {
            read_owned = read.key.clone().into_byte_exprs(cx);
            &read_owned
        };
        let write_owned;
        let write_bytes = if let Some(bytes) = write.key_bytes.as_deref() {
            bytes
        } else {
            write_owned = write.key.clone().into_byte_exprs(cx);
            &write_owned
        };
        let byte_equalities = read_bytes
            .iter()
            .zip(write_bytes)
            .map(|(read, write)| {
                let read = read.byte_term(cx, 31).unwrap_or_else(|| read.clone().low_byte(cx));
                let write = write.byte_term(cx, 31).unwrap_or_else(|| write.clone().low_byte(cx));
                SymBoolExpr::eq(cx, read, write)
            })
            .collect();
        SymBoolExpr::and(cx, byte_equalities)
    } else {
        SymBoolExpr::eq(cx, read.key.clone(), write.key.clone())
    }
}

fn storage_mapping_key_bytes_form_compact_word(bytes: &[SymExpr]) -> bool {
    bytes.iter().all(|byte| byte.as_const().is_some()) || word_from_extracted_bytes(bytes).is_some()
}

fn masked_expr_matches(candidate: &SymExprKind, target: &SymExpr) -> Option<U256> {
    match candidate {
        SymExprKind::BinOp(SymBinOp::And, left, right) if left == target => right.eval(),
        SymExprKind::BinOp(SymBinOp::And, left, right) if right == target => left.eval(),
        _ => None,
    }
}

fn context_forces_masked_expr(context: &[SymBoolExpr], target: &SymExpr, mask: U256) -> bool {
    context.iter().any(|condition| match condition.kind() {
        SymBoolExprKind::Cmp(SymCmpOp::Eq, left, right) => {
            (left == target && masked_expr_matches(right.kind(), target) == Some(mask))
                || (right == target && masked_expr_matches(left.kind(), target) == Some(mask))
        }
        SymBoolExprKind::And(values) => context_forces_masked_expr(values, target, mask),
        _ => false,
    })
}

pub(crate) fn concrete_expr_bytes(
    bytes: &[SymExpr],
    reason: &'static str,
) -> Result<Vec<u8>, SymbolicError> {
    bytes
        .iter()
        .map(|byte| match byte.as_const() {
            Some(value) => Ok(value.to::<u8>()),
            None => Err(SymbolicError::Unsupported(reason)),
        })
        .collect()
}

pub(crate) fn mask_low_bits(mask: U256) -> Option<usize> {
    let bits = mask.bit_len();
    (mask == mask_bits(U256::MAX, bits)).then_some(bits)
}

fn power_of_two_shift(value: U256) -> Option<usize> {
    if value <= U256::ONE || !value.is_power_of_two() {
        return None;
    }
    Some(value.bit_len() - 1)
}

pub(in crate::runtime::expr) fn low_masked_source(expr: &SymExpr, bits: usize) -> Option<&SymExpr> {
    match expr.kind() {
        // `a & low_mask => a`.
        SymExprKind::BinOp(SymBinOp::And, left, right)
            if right.as_const().and_then(mask_low_bits) == Some(bits) =>
        {
            Some(left)
        }
        _ => None,
    }
}

pub(in crate::runtime::expr) fn low_masked_source_any(expr: &SymExpr) -> Option<&SymExpr> {
    match expr.kind() {
        // `a & low_mask => a`.
        SymExprKind::BinOp(SymBinOp::And, left, right)
            if right.as_const().and_then(mask_low_bits).is_some() =>
        {
            Some(left)
        }
        _ => None,
    }
}

fn word_from_extracted_bytes(bytes: &[SymExpr]) -> Option<SymExpr> {
    if bytes.len() < 32 {
        return None;
    }

    let source = bytes
        .iter()
        .take(32)
        .enumerate()
        .find_map(|(idx, byte)| byte.extracted_byte_source(idx))?;

    for (idx, byte) in bytes.iter().take(32).enumerate() {
        if let Some(byte_source) = byte.extracted_byte_source(idx) {
            if byte_source != source {
                return None;
            }
            continue;
        }

        let byte = byte.as_const()?;
        if source.known_byte(idx) != Some(byte.to::<u8>()) {
            return None;
        }
    }
    Some(source)
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SymExpr {
    pub(in crate::runtime::expr) kind: HashConsed<SymExprKind>,
}

impl fmt::Debug for SymExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::runtime) enum SymExprKind {
    Const(U256),
    Var(Symbol),
    GasLeft(Symbol),
    Keccak { name: Symbol, len: SymExpr, bytes: Arc<[SymExpr]> },
    Hash { name: Symbol, algorithm: &'static str, bytes: Arc<[SymExpr]> },
    Not(SymExpr),
    BinOp(SymBinOp, SymExpr, SymExpr),
    TernOp(SymTernOp, SymExpr, SymExpr, SymExpr),
    Ite(SymBoolExpr, SymExpr, SymExpr),
}

impl SymExprKind {
    pub(in crate::runtime) const fn get_var(&self) -> Option<Symbol> {
        match self {
            Self::Var(symbol)
            | Self::GasLeft(symbol)
            | Self::Keccak { name: symbol, .. }
            | Self::Hash { name: symbol, .. } => Some(*symbol),
            _ => None,
        }
    }

    pub(in crate::runtime) const fn get_eval_var(&self) -> Option<Symbol> {
        match self {
            Self::Var(symbol) | Self::GasLeft(symbol) | Self::Hash { name: symbol, .. } => {
                Some(*symbol)
            }
            _ => None,
        }
    }
}

impl SymExpr {
    pub(in crate::runtime) fn kind(&self) -> &SymExprKind {
        self.kind.value()
    }

    #[cfg(test)]
    pub(crate) fn get_var_name<'a>(&self, cx: &'a SymCx) -> Option<&'a str> {
        self.kind().get_var().map(|symbol| cx.symbol_name(symbol))
    }

    #[cfg(test)]
    pub(crate) fn is_keccak(&self) -> bool {
        matches!(self.kind(), SymExprKind::Keccak { .. })
    }

    #[cfg(test)]
    pub(crate) fn keccak_len_and_byte_count(&self) -> Option<(&Self, usize)> {
        match self.kind() {
            SymExprKind::Keccak { len, bytes, .. } => Some((len, bytes.len())),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn hash_algorithm(&self) -> Option<&'static str> {
        match self.kind() {
            SymExprKind::Hash { algorithm, .. } => Some(algorithm),
            _ => None,
        }
    }

    pub(in crate::runtime) fn into_kind(self) -> SymExprKind {
        self.kind.into_value()
    }

    pub(in crate::runtime) fn from_kind(cx: &mut SymCx, kind: SymExprKind) -> Self {
        cx.mk_expr_kind(kind)
    }

    pub(crate) fn zero(cx: &mut SymCx) -> Self {
        Self::constant(cx, U256::ZERO)
    }

    pub(crate) fn one(cx: &mut SymCx) -> Self {
        Self::constant(cx, U256::ONE)
    }

    pub(crate) fn constant(cx: &mut SymCx, value: U256) -> Self {
        if value.is_zero() {
            return cx.cached_zero();
        }
        if value == U256::ONE {
            return cx.cached_one();
        }
        Self::from_kind(cx, SymExprKind::Const(value))
    }

    pub(crate) fn var(cx: &mut SymCx, name: &str) -> Self {
        let symbol = cx.intern(name);
        Self::get_var(cx, symbol)
    }

    pub(crate) fn get_var(cx: &mut SymCx, symbol: Symbol) -> Self {
        Self::from_kind(cx, SymExprKind::Var(symbol))
    }

    pub(crate) fn gas_left(cx: &mut SymCx, id: usize) -> Self {
        let symbol = cx.intern(&format!("gasleft_{id}"));
        Self::from_kind(cx, SymExprKind::GasLeft(symbol))
    }

    pub(crate) fn not(cx: &mut SymCx, value: Self) -> Self {
        match value.kind() {
            SymExprKind::Const(value) => Self::constant(cx, !*value),
            SymExprKind::Not(value) => value.clone(),
            _ => Self::from_kind(cx, SymExprKind::Not(value)),
        }
    }

    pub(crate) fn binop(cx: &mut SymCx, binop: SymBinOp, left: Self, right: Self) -> Self {
        match binop {
            SymBinOp::Add => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const + const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 + a => a`.
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                // `a + 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `bool_word(c) + MAX => ite(c, 0, MAX)`.
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if *value == U256::MAX
                        && let Some(condition) = if left.as_const() == Some(U256::MAX) {
                            right.bitwise_bool_word_condition(cx)
                        } else {
                            left.bitwise_bool_word_condition(cx)
                        } =>
                {
                    let zero = Self::zero(cx);
                    let max = Self::constant(cx, U256::MAX);
                    Self::ite(cx, condition, zero, max)
                }
                // `one_bit_word(c) + k => ite(c, k + 1, k)`.
                (SymExprKind::Const(value), _)
                    if let Some(condition) = right.bitwise_bool_word_condition(cx) =>
                {
                    let incremented = Self::constant(cx, value.wrapping_add(U256::ONE));
                    let value = Self::constant(cx, *value);
                    Self::ite(cx, condition, incremented, value)
                }
                (_, SymExprKind::Const(value))
                    if let Some(condition) = left.bitwise_bool_word_condition(cx) =>
                {
                    let incremented = Self::constant(cx, value.wrapping_add(U256::ONE));
                    let value = Self::constant(cx, *value);
                    Self::ite(cx, condition, incremented, value)
                }
                _ => {
                    let (left, right) = Self::ordered_commutative_operands(left, right);
                    if let Some(value) = Self::add_with_const_ite(cx, &left, &right) {
                        value
                    } else if let Some(value) = Self::add_with_const_ite(cx, &right, &left) {
                        value
                    } else {
                        Self::from_kind(cx, SymExprKind::BinOp(binop, left, right))
                    }
                }
            },
            SymBinOp::Sub => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const - const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a - 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `a - a => 0`.
                _ if left == right => Self::zero(cx),
                // `bool_word(c) - 1 => ite(c, 0, MAX)`.
                (_, SymExprKind::Const(value))
                    if *value == U256::ONE
                        && let Some(condition) = left.bitwise_bool_word_condition(cx) =>
                {
                    let zero = Self::zero(cx);
                    let max = Self::constant(cx, U256::MAX);
                    Self::ite(cx, condition, zero, max)
                }
                // `k - bool_word(c) => ite(c, k - 1, k)`.
                (SymExprKind::Const(value), _)
                    if let Some(condition) = right.bitwise_bool_word_condition(cx) =>
                {
                    let decremented = Self::constant(cx, value.wrapping_sub(U256::ONE));
                    let value = Self::constant(cx, *value);
                    Self::ite(cx, condition, decremented, value)
                }
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymBinOp::Mul => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const * const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 * a => 0`.
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    Self::zero(cx)
                }
                // `1 * a => a`.
                (SymExprKind::Const(value), _) if *value == U256::ONE => right,
                // `a * 1 => a`.
                (_, SymExprKind::Const(value)) if *value == U256::ONE => left,
                _ => {
                    let (left, right) = Self::ordered_commutative_operands(left, right);
                    if let Some(condition) = left.direct_bool_word_condition(cx) {
                        // `bool_word(c) * a => ite(c, a, 0)`.
                        let zero = Self::zero(cx);
                        Self::ite(cx, condition, right, zero)
                    } else if let Some(condition) = right.direct_bool_word_condition(cx) {
                        // `a * bool_word(c) => ite(c, a, 0)`.
                        let zero = Self::zero(cx);
                        Self::ite(cx, condition, left, zero)
                    } else if let Some(shift) = right.as_const().and_then(power_of_two_shift) {
                        // `a * 2**n => a << n`.
                        let shift = Self::constant(cx, U256::from(shift));
                        Self::binop(cx, SymBinOp::Shl, left, shift)
                    } else {
                        Self::from_kind(cx, SymExprKind::BinOp(binop, left, right))
                    }
                }
            },
            SymBinOp::UDiv | SymBinOp::SDiv => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const / const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a / 0 => 0`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::zero(cx),
                // `a / 1 => a`.
                (_, SymExprKind::Const(value)) if *value == U256::ONE => left,
                // `(a - (a & (2**n - 1))) u/ 2**n => a >> n`.
                (
                    SymExprKind::BinOp(SymBinOp::Sub, value, low_bits),
                    SymExprKind::Const(divisor),
                ) if binop == SymBinOp::UDiv
                    && let Some(shift) = power_of_two_shift(*divisor)
                    && low_masked_source(low_bits, shift) == Some(value) =>
                {
                    let shift = Self::constant(cx, U256::from(shift));
                    Self::binop(cx, SymBinOp::Shr, value.clone(), shift)
                }
                // `a u/ 2**n => a >> n`.
                (_, SymExprKind::Const(divisor))
                    if binop == SymBinOp::UDiv
                        && let Some(shift) = power_of_two_shift(*divisor) =>
                {
                    let shift = Self::constant(cx, U256::from(shift));
                    Self::binop(cx, SymBinOp::Shr, left, shift)
                }
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymBinOp::URem | SymBinOp::SRem => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const % const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a % 0 => 0`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::zero(cx),
                // `a % 1 => 0`.
                (_, SymExprKind::Const(value)) if *value == U256::ONE => Self::zero(cx),
                // `a u% 2**n => a & (2**n - 1)`.
                (_, SymExprKind::Const(divisor))
                    if binop == SymBinOp::URem
                        && let Some(bits) = power_of_two_shift(*divisor) =>
                {
                    Self::and_const(cx, left, mask_bits(U256::MAX, bits))
                }
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymBinOp::And => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const & const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 & a => 0`.
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    Self::zero(cx)
                }
                // `MAX & a => a`.
                (SymExprKind::Const(value), _) if *value == U256::MAX => right,
                // `a & MAX => a`.
                (_, SymExprKind::Const(value)) if *value == U256::MAX => left,
                // `a & a => a`.
                _ if left == right => left,
                (SymExprKind::Const(mask), _) => Self::and_const(cx, right, *mask),
                (_, SymExprKind::Const(mask)) => Self::and_const(cx, left, *mask),
                _ => Self::commutative_binop(cx, binop, left, right),
            },
            SymBinOp::Or => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const | const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 | a => a`.
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                // `a | 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `a | a => a`.
                _ if left == right => left,
                _ if let Some(value) = Self::or_with_absorbing_ite(cx, &left, right.clone()) => {
                    value
                }
                _ if let Some(value) = Self::or_with_absorbing_ite(cx, &right, left.clone()) => {
                    value
                }
                _ => Self::or(cx, left, right),
            },
            SymBinOp::Xor => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const ^ const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 ^ a => a`.
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                // `a ^ 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `a ^ a => 0`.
                _ if left == right => Self::zero(cx),
                _ => {
                    let (left, right) = Self::ordered_commutative_operands(left, right);
                    // `a ^ (a ^ b) => b`.
                    if let Some(value) = Self::xor_with_shared_operand(&left, &right)
                        .or_else(|| Self::xor_with_shared_operand(&right, &left))
                    {
                        value
                    // `a ^ ((a ^ b) * bool_word(c)) => ite(c, b, a)`.
                    } else if let Some(value) = Self::xor_with_bool_select(cx, &left, &right) {
                        value
                    } else if let Some(value) = Self::xor_with_bool_select(cx, &right, &left) {
                        value
                    // `a ^ ite(c, b, 0) => ite(c, a ^ b, a)`.
                    } else if let Some(value) = Self::xor_with_zero_ite(cx, &left, &right) {
                        value
                    } else if let Some(value) = Self::xor_with_zero_ite(cx, &right, &left) {
                        value
                    } else {
                        Self::from_kind(cx, SymExprKind::BinOp(binop, left, right))
                    }
                }
            },
            SymBinOp::Shl => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const << const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a << 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `0 << a => 0`.
                (SymExprKind::Const(value), _) if value.is_zero() => Self::zero(cx),
                // `a << 256 => 0`.
                (_, SymExprKind::Const(value)) if *value >= U256::from(256) => Self::zero(cx),
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymBinOp::Shr => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const >> const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a >> 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `0 >> a => 0`.
                (SymExprKind::Const(value), _) if value.is_zero() => Self::zero(cx),
                (_, SymExprKind::Const(value)) => Self::shr_const(cx, left, *value),
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymBinOp::Sar => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const >>s const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a >>s 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
        }
    }

    pub(crate) fn ternop(
        cx: &mut SymCx,
        ternop: SymTernOp,
        left: Self,
        right: Self,
        modulus: Self,
    ) -> Self {
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus)) if modulus.is_zero() || *modulus == U256::ONE => {
                // `addmod/mulmod(a, b, 0) => 0`.
                Self::zero(cx)
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                // `addmod/mulmod(const, const, const) => const`.
                Self::constant(cx, ternop.eval(*left, *right, *modulus))
            }
            // `addmod/mulmod(a, b, 2**n) => op(a, b) & (2**n - 1)`.
            (_, _, SymExprKind::Const(modulus))
                if let Some(bits) = power_of_two_shift(*modulus) =>
            {
                let binop = match ternop {
                    SymTernOp::AddMod => SymBinOp::Add,
                    SymTernOp::MulMod => SymBinOp::Mul,
                };
                let value = Self::binop(cx, binop, left, right);
                Self::and_const(cx, value, mask_bits(U256::MAX, bits))
            }
            _ => {
                // `addmod/mulmod(a, b, m) => addmod/mulmod(ordered(a, b), m)`.
                let (left, right) = Self::ordered_commutative_operands(left, right);
                Self::from_kind(cx, SymExprKind::TernOp(ternop, left, right, modulus))
            }
        }
    }

    pub(crate) fn ite(
        cx: &mut SymCx,
        condition: SymBoolExpr,
        then_expr: Self,
        else_expr: Self,
    ) -> Self {
        match condition.as_const() {
            // `ite(true, a, b) => a`.
            Some(true) => then_expr,
            // `ite(false, a, b) => b`.
            Some(false) => else_expr,
            // `ite(c, a, a) => a`.
            None if then_expr == else_expr => then_expr,
            // `ite(a == 0, 0, a / a) => a != 0`.
            None if then_expr.as_const().is_some_and(|value| value.is_zero())
                && Self::self_div_expr_matches_zero_check(&condition, &else_expr) =>
            {
                let condition = condition.not(cx);
                Self::bool_word(cx, condition)
            }
            // `ite(c, 1, bool_word(c)) => bool_word(c)`.
            None if then_expr.as_const() == Some(U256::ONE)
                && else_expr.bool_word_condition().as_ref() == Some(&condition) =>
            {
                else_expr
            }
            // `ite(c, bool_word(c), 0) => bool_word(c)`.
            None if else_expr.as_const().is_some_and(|value| value.is_zero())
                && then_expr.bool_word_condition().as_ref() == Some(&condition) =>
            {
                then_expr
            }
            None => Self::from_kind(cx, SymExprKind::Ite(condition, then_expr, else_expr)),
        }
    }

    pub(crate) fn bool_word(cx: &mut SymCx, value: SymBoolExpr) -> Self {
        let one = Self::one(cx);
        let zero = Self::zero(cx);
        Self::ite(cx, value, one, zero)
    }

    fn self_div_expr_matches_zero_check(cond: &SymBoolExpr, expr: &Self) -> bool {
        let Some(zero_operand) = cond.zero_check_operand() else { return false };
        let Some((numerator, denominator)) = expr.udiv_operands() else { return false };
        numerator == zero_operand && denominator == zero_operand
    }

    pub(crate) fn keccak_symbol(cx: &mut SymCx, name: Symbol, len: Self, bytes: Vec<Self>) -> Self {
        Self::from_kind(cx, SymExprKind::Keccak { name, len, bytes: bytes.into() })
    }

    pub(crate) fn hash_symbol(
        cx: &mut SymCx,
        name: Symbol,
        algorithm: &'static str,
        bytes: Vec<Self>,
    ) -> Self {
        Self::from_kind(cx, SymExprKind::Hash { name, algorithm, bytes: bytes.into() })
    }

    fn or(cx: &mut SymCx, left: Self, right: Self) -> Self {
        if let Some(rebuilt) = Self::rebuild_from_or_terms(&left, &right) {
            // `byte_parts(a) | byte_parts(a) => a`.
            return rebuilt;
        }
        Self::commutative_binop(cx, SymBinOp::Or, left, right)
    }

    fn or_with_absorbing_ite(cx: &mut SymCx, conditional: &Self, other: Self) -> Option<Self> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = conditional.kind() else {
            return None;
        };
        if then_expr.as_const().is_some_and(|value| value.is_zero())
            && else_expr.as_const() == Some(U256::MAX)
        {
            return Some(Self::ite(cx, condition.clone(), other, else_expr.clone()));
        }
        if then_expr.as_const() == Some(U256::MAX)
            && else_expr.as_const().is_some_and(|value| value.is_zero())
        {
            return Some(Self::ite(cx, condition.clone(), then_expr.clone(), other));
        }
        None
    }

    fn add_with_const_ite(cx: &mut SymCx, other: &Self, conditional: &Self) -> Option<Self> {
        if other.as_const().is_some() {
            return None;
        }
        let SymExprKind::Ite(condition, then_expr, else_expr) = conditional.kind() else {
            return None;
        };
        if then_expr.as_const().is_none() || else_expr.as_const().is_none() {
            return None;
        }
        if !Self::duplicating_branchless_rewrite_fits(other, conditional) {
            return None;
        }
        let then_expr = Self::binop(cx, SymBinOp::Add, other.clone(), then_expr.clone());
        let else_expr = Self::binop(cx, SymBinOp::Add, other.clone(), else_expr.clone());
        Some(Self::ite(cx, condition.clone(), then_expr, else_expr))
    }

    fn xor_with_bool_select(cx: &mut SymCx, base: &Self, selector: &Self) -> Option<Self> {
        let SymExprKind::BinOp(SymBinOp::Mul, left, right) = selector.kind() else { return None };
        let (condition_word, selected) = match left.kind() {
            SymExprKind::BinOp(SymBinOp::Xor, delta_left, delta_right) if delta_left == base => {
                (right, delta_right.clone())
            }
            SymExprKind::BinOp(SymBinOp::Xor, delta_left, delta_right) if delta_right == base => {
                (right, delta_left.clone())
            }
            _ => match right.kind() {
                SymExprKind::BinOp(SymBinOp::Xor, delta_left, delta_right)
                    if delta_left == base =>
                {
                    (left, delta_right.clone())
                }
                SymExprKind::BinOp(SymBinOp::Xor, delta_left, delta_right)
                    if delta_right == base =>
                {
                    (left, delta_left.clone())
                }
                _ => return None,
            },
        };
        let condition = condition_word.bitwise_bool_word_condition(cx)?;
        Some(Self::ite(cx, condition, selected, base.clone()))
    }

    fn xor_with_shared_operand(base: &Self, nested: &Self) -> Option<Self> {
        let SymExprKind::BinOp(SymBinOp::Xor, left, right) = nested.kind() else { return None };
        if left == base {
            Some(right.clone())
        } else if right == base {
            Some(left.clone())
        } else {
            None
        }
    }

    fn xor_with_zero_ite(cx: &mut SymCx, base: &Self, conditional: &Self) -> Option<Self> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = conditional.kind() else {
            return None;
        };
        if then_expr.as_const().is_some_and(|value| value.is_zero()) {
            if !Self::duplicating_branchless_rewrite_fits(base, conditional) {
                return None;
            }
            let selected = Self::binop(cx, SymBinOp::Xor, base.clone(), else_expr.clone());
            return Some(Self::ite(cx, condition.clone(), base.clone(), selected));
        }
        if else_expr.as_const().is_some_and(|value| value.is_zero()) {
            if !Self::duplicating_branchless_rewrite_fits(base, conditional) {
                return None;
            }
            let selected = Self::binop(cx, SymBinOp::Xor, base.clone(), then_expr.clone());
            return Some(Self::ite(cx, condition.clone(), selected, base.clone()));
        }
        None
    }

    /// Checks the occurrence cost of a rewrite that places one operand in both ITE arms.
    ///
    /// Hash-consing keeps the stored DAG compact, but solver normalization folds occurrences and
    /// would revisit a shared operand twice after every rewrite. Count that unfolded result before
    /// constructing it so a linear series of branchless operations cannot become exponential.
    fn duplicating_branchless_rewrite_fits(operand: &Self, conditional: &Self) -> bool {
        let mut counter = UnfoldedNodeCounter::new();
        let Some(operand_nodes) = counter.expr_nodes(operand) else {
            return false;
        };
        let Some(conditional_nodes) = counter.expr_nodes(conditional) else {
            return false;
        };

        operand_nodes
            .checked_mul(2)
            .and_then(|duplicated| conditional_nodes.checked_add(duplicated))
            // The rewritten ITE adds at most two operation wrappers around the original arms.
            .and_then(|nodes| nodes.checked_add(2))
            .is_some_and(|nodes| nodes <= MAX_BRANCHLESS_REWRITE_UNFOLDED_NODES)
    }

    fn commutative_binop(cx: &mut SymCx, op: SymBinOp, left: Self, right: Self) -> Self {
        // `a + b => b + a`.
        let (left, right) = Self::ordered_commutative_operands(left, right);
        Self::from_kind(cx, SymExprKind::BinOp(op, left, right))
    }

    pub(in crate::runtime::expr) fn ordered_commutative_operands(
        left: Self,
        right: Self,
    ) -> (Self, Self) {
        match left.complexity().cmp(&right.complexity()) {
            // Put less complex operands, like constants, on the RHS.
            std::cmp::Ordering::Less => (right, left),
            std::cmp::Ordering::Greater => (left, right),
            std::cmp::Ordering::Equal if right.kind.stable_hash_cmp(&left.kind).is_lt() => {
                (right, left)
            }
            std::cmp::Ordering::Equal => (left, right),
        }
    }

    /// Canonically orders factors that belong to the same symbolic context.
    ///
    /// Polynomial normalization only needs a stable order within one context, where hash-consed
    /// identity is both total and substantially cheaper than rendering structural string keys.
    pub(in crate::runtime) fn sort_interned_factors(factors: &mut [Self]) {
        factors.sort_unstable_by(|left, right| left.kind.identity_cmp(&right.kind));
    }

    fn complexity(&self) -> usize {
        match self.kind() {
            SymExprKind::Const(_) => 0,
            SymExprKind::Not(_) => 1,
            SymExprKind::BinOp(..) => 2,
            SymExprKind::TernOp(..) => 3,
            _ => 4,
        }
    }

    fn and_const(cx: &mut SymCx, expr: Self, mask: U256) -> Self {
        if mask.is_zero() {
            // `a & 0 => 0`.
            return Self::zero(cx);
        }
        if mask == U256::MAX {
            // `a & MAX => a`.
            return expr;
        }

        match expr.kind() {
            // `const & mask => const`.
            SymExprKind::Const(value) => Self::constant(cx, *value & mask),
            SymExprKind::BinOp(SymBinOp::Or, left, right) => {
                // `(a | b) & mask => (a & mask) | (b & mask)`.
                let left = Self::and_const(cx, left.clone(), mask);
                let right = Self::and_const(cx, right.clone(), mask);
                Self::binop(cx, SymBinOp::Or, left, right)
            }
            SymExprKind::BinOp(SymBinOp::Shl, _, shift)
                if mask_low_bits(mask).is_some_and(|bits| {
                    shift
                        .as_const()
                        .and_then(|shift| usize::try_from(shift).ok())
                        .is_some_and(|shift| bits <= shift)
                }) =>
            {
                // `(a << n) & low_mask(n) => 0`.
                Self::zero(cx)
            }
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                if right.as_const() == Some(mask) {
                    // `(a & mask) & mask => a & mask`.
                    Self::and_const(cx, left.clone(), mask)
                } else if left == right {
                    // `(a & a) & mask => a & mask`.
                    Self::and_const(cx, left.clone(), mask)
                } else {
                    let mask = Self::constant(cx, mask);
                    Self::from_kind(cx, SymExprKind::BinOp(SymBinOp::And, expr, mask))
                }
            }
            _ => {
                let mask = Self::constant(cx, mask);
                Self::from_kind(cx, SymExprKind::BinOp(SymBinOp::And, expr, mask))
            }
        }
    }

    fn shr_const(cx: &mut SymCx, expr: Self, shift: U256) -> Self {
        if shift.is_zero() {
            // `a >> 0 => a`.
            return expr;
        }
        if shift >= U256::from(256) {
            // `a >> 256 => 0`.
            return Self::zero(cx);
        }

        let shift = usize::try_from(shift).expect("shift is less than 256");
        if expr.unsigned_bits() <= shift {
            // `small(a) >> bits(a) => 0`.
            return Self::zero(cx);
        }

        if let SymExprKind::BinOp(SymBinOp::Shl, inner, left_shift) = expr.kind()
            && left_shift.as_const() == Some(U256::from(shift))
            && inner.unsigned_bits() <= 256 - shift
        {
            // `(a << n) >> n => a`.
            return inner.clone();
        }

        if let SymExprKind::BinOp(SymBinOp::Or, left, right) = expr.kind() {
            // Distribute only when it collapses part of the OR; expanding broad
            // bit-smearing chains eagerly makes SMT CSE much larger.
            let left = Self::shr_const(cx, left.clone(), U256::from(shift));
            let right = Self::shr_const(cx, right.clone(), U256::from(shift));
            if left.as_const().is_some_and(|value| value.is_zero()) {
                return right;
            }
            if right.as_const().is_some_and(|value| value.is_zero()) {
                return left;
            }
        }

        let shift = Self::constant(cx, U256::from(shift));
        Self::from_kind(cx, SymExprKind::BinOp(SymBinOp::Shr, expr, shift))
    }

    fn rebuild_from_or_terms(left: &Self, right: &Self) -> Option<Self> {
        let mut terms = Vec::new();
        left.push_or_terms(&mut terms);
        right.push_or_terms(&mut terms);
        Self::rebuild_from_extracted_byte_terms(&terms)
            .or_else(|| Self::rebuild_from_shifted_word_fragments(&terms))
    }

    pub(in crate::runtime) fn push_or_terms<'a>(&'a self, terms: &mut Vec<&'a Self>) {
        match self.kind() {
            SymExprKind::BinOp(SymBinOp::Or, left, right) => {
                left.push_or_terms(terms);
                right.push_or_terms(terms);
            }
            _ => terms.push(self),
        }
    }

    fn rebuild_from_extracted_byte_terms(terms: &[&Self]) -> Option<Self> {
        if terms.len() <= 1 {
            return None;
        }

        let mut source = None;
        let mut seen = [false; 32];
        for term in terms {
            if term.as_const().is_some_and(|value| value.is_zero()) {
                continue;
            }
            let (term_source, index) = term.extracted_shifted_byte_term()?;
            match &source {
                Some(source) if source != &term_source => return None,
                Some(_) => {}
                None => source = Some(term_source),
            }
            seen[index] = true;
        }

        let source = source?;
        for (index, seen) in seen.into_iter().enumerate() {
            if !seen && source.known_byte(index) != Some(0) {
                return None;
            }
        }
        Some(source)
    }

    fn extracted_shifted_byte_term(&self) -> Option<(Self, usize)> {
        match self.kind() {
            SymExprKind::BinOp(SymBinOp::Shl, byte, shift) => {
                let shift = shift.as_const()?;
                let Ok(shift) = usize::try_from(shift) else { return None };
                if shift % 8 != 0 || shift > 248 {
                    return None;
                }
                let index = 31 - shift / 8;
                let source = byte.extracted_unshifted_byte_source(index)?;
                Some((source, index))
            }
            _ => self.extracted_unshifted_byte_source(31).map(|source| (source, 31)),
        }
    }

    fn extracted_unshifted_byte_source(&self, index: usize) -> Option<Self> {
        let expr = self.strip_low_byte_mask();
        if index == 31 {
            return Some(expr.clone());
        }
        let SymExprKind::BinOp(SymBinOp::Shr, source, shift) = expr.kind() else { return None };
        let shift = shift.as_const()?;
        (shift == U256::from((31 - index) * 8)).then(|| source.clone())
    }

    fn rebuild_from_shifted_word_fragments(terms: &[&Self]) -> Option<Self> {
        if terms.len() != 2 {
            return None;
        }

        let left_low = terms[0].low_word_fragment();
        let right_low = terms[1].low_word_fragment();
        let left_high = terms[0].shifted_high_word_fragment();
        let right_high = terms[1].shifted_high_word_fragment();
        match (left_low, right_low, left_high, right_high) {
            (Some((low_source, low_bits)), None, None, Some((high_source, high_bits)))
            | (None, Some((low_source, low_bits)), Some((high_source, high_bits)), None)
                if low_source == high_source && low_bits == high_bits =>
            {
                Some(low_source)
            }
            _ => None,
        }
    }

    fn low_word_fragment(&self) -> Option<(Self, usize)> {
        let SymExprKind::BinOp(SymBinOp::And, left, right) = self.kind() else { return None };
        let mask = right.as_const()?;
        mask_low_bits(mask).map(|bits| (left.clone(), bits))
    }

    fn shifted_high_word_fragment(&self) -> Option<(Self, usize)> {
        let SymExprKind::BinOp(SymBinOp::Shl, value, shift) = self.kind() else { return None };
        let bits = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
        if bits == 0 || bits >= 256 {
            return None;
        }

        let (source, source_shift, width) = value.shifted_low_fragment_source()?;
        (source_shift == bits && width == 256 - bits).then_some((source, bits))
    }

    fn shifted_low_fragment_source(&self) -> Option<(Self, usize, usize)> {
        let SymExprKind::BinOp(SymBinOp::And, left, right) = self.kind() else { return None };
        let mask = right.as_const()?;
        Self::shifted_low_fragment_source_with_mask(left, mask)
    }

    fn shifted_low_fragment_source_with_mask(
        value: &Self,
        mask: U256,
    ) -> Option<(Self, usize, usize)> {
        let width = mask_low_bits(mask)?;
        match value.kind() {
            SymExprKind::BinOp(SymBinOp::Shr, source, shift) => {
                let shift = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
                Some((source.clone(), shift, width))
            }
            _ => Some((value.clone(), 0, width)),
        }
    }

    pub(crate) fn low_byte(self, cx: &mut SymCx) -> Self {
        if let Some(word) = self.as_const() {
            return Self::constant(cx, U256::from(word.to::<u8>()));
        }
        let mask = Self::constant(cx, U256::from(0xff));
        Self::binop(cx, SymBinOp::And, self, mask)
    }

    pub(crate) fn into_byte_exprs(self, cx: &mut SymCx) -> Vec<Self> {
        SymBytes::word(cx, self).materialize(cx)
    }

    pub(crate) fn into_bytes(self, cx: &mut SymCx) -> SymBytes {
        SymBytes::word(cx, self)
    }

    pub(crate) fn from_bytes(cx: &mut SymCx, bytes: impl IntoIterator<Item = Self>) -> Self {
        let bytes = bytes.into_iter().collect::<Vec<_>>();
        if let Ok(concrete) = concrete_expr_bytes(&bytes, "symbolic word bytes") {
            let mut word = [0u8; 32];
            for (idx, byte) in concrete.into_iter().take(32).enumerate() {
                word[idx] = byte;
            }
            return Self::constant(cx, U256::from_be_bytes(word));
        }

        if let Some(expr) = word_from_extracted_bytes(&bytes) {
            return expr;
        }

        let mut expr = Self::zero(cx);
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            let shift = (31 - idx) * 8;
            let byte = byte.low_byte(cx);
            let byte = if shift == 0 {
                byte
            } else {
                let shift = Self::constant(cx, U256::from(shift));
                Self::binop(cx, SymBinOp::Shl, byte, shift)
            };
            expr = Self::binop(cx, SymBinOp::Or, expr, byte);
        }
        expr
    }

    pub(crate) fn as_const(&self) -> Option<U256> {
        match self.kind() {
            SymExprKind::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn eval(&self) -> Option<U256> {
        self.eval_model_if_complete(&NoopModel).ok().flatten()
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<U256, SymbolicError> {
        ModelEvaluator::new(model).eval_word(self)
    }

    pub(crate) fn eval_model_if_complete<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Option<U256>, SymbolicError> {
        let mut vars = SymbolicVars::default();
        self.collect_eval_vars(&mut vars);
        if vars.iter().copied().all(|var| model.contains_name(var)) {
            self.eval_model(model).map(Some)
        } else {
            Ok(None)
        }
    }

    pub(crate) fn assign_model_value(&self, model: &mut SymbolicModel, value: U256) -> bool {
        match self.kind() {
            SymExprKind::Const(existing) => *existing == value,
            SymExprKind::Var(var) => {
                if let Some(existing) = model.get(var) {
                    *existing == value
                } else {
                    model.insert(*var, value);
                    true
                }
            }
            SymExprKind::GasLeft(symbol) => {
                if let Some(existing) = model.get(symbol) {
                    *existing == value
                } else {
                    model.insert(*symbol, value);
                    true
                }
            }
            _ => false,
        }
    }

    pub(crate) fn bool_word_condition(&self) -> Option<SymBoolExpr> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() else {
            return None;
        };
        Self::bool_word_condition_from_parts(condition, then_expr, else_expr)
    }

    pub(in crate::runtime) fn bitwise_bool_word_condition(
        &self,
        cx: &mut SymCx,
    ) -> Option<SymBoolExpr> {
        let mut pending = vec![self.clone()];
        let mut seen_words = HashSet::<Self>::default();
        let mut leaf_conditions = IndexSet::<SymBoolExpr>::default();
        let mut bit_widths = HashMap::default();
        let mut remaining = MAX_BITWISE_BOOL_WORD_VISITS;
        while let Some(word) = pending.pop() {
            if !seen_words.insert(word.clone()) {
                continue;
            }
            if remaining == 0 {
                return None;
            }
            remaining -= 1;

            if let Some(condition) = word.direct_bool_word_condition(cx) {
                leaf_conditions.insert(condition);
                continue;
            }
            if let SymExprKind::BinOp(SymBinOp::Or, left, right) = word.kind() {
                pending.push(right.clone());
                pending.push(left.clone());
                continue;
            }

            if word.unsigned_bits_cached(&mut bit_widths, &mut remaining) == Some(1) {
                let zero = Self::zero(cx);
                let (word, zero) = Self::ordered_commutative_operands(word, zero);
                let zero_check =
                    SymBoolExpr::from_kind(cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, word, zero));
                leaf_conditions.insert(zero_check.not(cx));
                continue;
            }
            return None;
        }

        Some(SymBoolExpr::or(cx, leaf_conditions.into_iter().collect()))
    }

    /// Returns the condition represented by a direct `0`/`1` ITE, preserving its polarity.
    ///
    /// Unlike [`Self::bitwise_bool_word_condition`], this deliberately does not infer a boolean
    /// word from arbitrary one-bit expressions. Callers that run during expression construction
    /// use this bounded structural check to avoid recursively sizing a shared expression DAG.
    fn direct_bool_word_condition(&self, cx: &mut SymCx) -> Option<SymBoolExpr> {
        let SymExprKind::Ite(condition, then_expr, else_expr) = self.kind() else {
            return None;
        };
        match (then_expr.as_const(), else_expr.as_const()) {
            (Some(then_value), Some(else_value))
                if then_value == U256::ONE && else_value.is_zero() =>
            {
                Some(condition.clone())
            }
            (Some(then_value), Some(else_value))
                if then_value.is_zero() && else_value == U256::ONE =>
            {
                Some(condition.clone().not(cx))
            }
            _ => None,
        }
    }

    fn bool_word_condition_from_parts(
        condition: &SymBoolExpr,
        then_expr: &Self,
        else_expr: &Self,
    ) -> Option<SymBoolExpr> {
        match (then_expr.as_const(), else_expr.as_const()) {
            (Some(then_value), Some(else_value))
                if then_value == U256::ONE && else_value.is_zero() =>
            {
                Some(condition.clone())
            }
            (Some(then_value), Some(else_value))
                if then_value.is_zero() && else_value == U256::ONE =>
            {
                None
            }
            _ => None,
        }
    }

    pub(crate) fn truth(&self) -> Option<bool> {
        self.as_const().map(|value| !value.is_zero())
    }

    pub(crate) fn into_zero_bool(self, cx: &mut SymCx) -> SymBoolExpr {
        match self.kind() {
            SymExprKind::Const(value) => SymBoolExpr::constant(cx, value.is_zero()),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                match Self::bool_word_condition_from_parts(condition, then_expr, else_expr) {
                    Some(condition) => SymBoolExpr::not_bool(cx, condition),
                    None => {
                        let zero = Self::zero(cx);
                        SymBoolExpr::eq(cx, self, zero)
                    }
                }
            }
            _ => {
                let zero = Self::zero(cx);
                SymBoolExpr::eq(cx, self, zero)
            }
        }
    }

    pub(crate) fn nonzero_bool(self, cx: &mut SymCx) -> SymBoolExpr {
        let zero = self.into_zero_bool(cx);
        SymBoolExpr::not_bool(cx, zero)
    }

    pub(crate) fn as_const_or(&self, reason: &'static str) -> Result<U256, SymbolicError> {
        self.as_const().ok_or(SymbolicError::Unsupported(reason))
    }

    pub(crate) fn as_usize_or(&self, reason: &'static str) -> Result<usize, SymbolicError> {
        let value = self.as_const_or(reason)?;
        usize::try_from(value).map_err(|_| SymbolicError::Unsupported(reason))
    }

    pub(crate) fn contains_keccak(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Keccak { .. }))
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::GasLeft(_)))
    }

    pub(crate) fn contains_udiv(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::BinOp(SymBinOp::UDiv, _, _)))
    }

    pub(crate) fn contains_ite(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Ite(_, _, _)))
    }

    pub(in crate::runtime) fn udiv_operands(&self) -> Option<(&Self, &Self)> {
        match self.kind() {
            SymExprKind::BinOp(SymBinOp::UDiv, numerator, denominator) => {
                Some((numerator, denominator))
            }
            _ => None,
        }
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit(&mut |expr| {
            if let Some(var) = expr.kind().get_eval_var() {
                vars.insert(var);
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn known_byte(&self, index: usize) -> Option<u8> {
        debug_assert!(index < 32);
        match self.kind() {
            SymExprKind::Const(value) => Some(value.to_be_bytes::<32>()[index]),
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => None,
            SymExprKind::Not(value) => value.known_byte(index).map(|byte| !byte),
            SymExprKind::Ite(_, then_expr, else_expr) => {
                let then_byte = then_expr.known_byte(index)?;
                let else_byte = else_expr.known_byte(index)?;
                (then_byte == else_byte).then_some(then_byte)
            }
            SymExprKind::BinOp(op, left, right) => match op {
                SymBinOp::And => match (left.known_byte(index), right.known_byte(index)) {
                    (Some(left), Some(right)) => Some(left & right),
                    (Some(0), _) | (_, Some(0)) => Some(0),
                    _ => None,
                },
                SymBinOp::Or => Some(left.known_byte(index)? | right.known_byte(index)?),
                SymBinOp::Xor => Some(left.known_byte(index)? ^ right.known_byte(index)?),
                SymBinOp::Shl => {
                    let shift = right.as_const()?;
                    if shift >= U256::from(256) {
                        return Some(0);
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let source_index = index + shift / 8;
                    if source_index >= 32 { Some(0) } else { left.known_byte(source_index) }
                }
                SymBinOp::Shr => {
                    let shift = right.as_const()?;
                    if shift >= U256::from(256) {
                        return Some(0);
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let byte_shift = shift / 8;
                    if index < byte_shift { Some(0) } else { left.known_byte(index - byte_shift) }
                }
                SymBinOp::Add
                | SymBinOp::Sub
                | SymBinOp::Mul
                | SymBinOp::UDiv
                | SymBinOp::URem
                | SymBinOp::SDiv
                | SymBinOp::SRem
                | SymBinOp::Sar => None,
            },
            SymExprKind::TernOp(_, _, _, _) => None,
        }
    }

    pub(crate) fn known_word(&self) -> Option<U256> {
        let mut word = [0u8; 32];
        for (idx, byte) in word.iter_mut().enumerate() {
            *byte = self.known_byte(idx)?;
        }
        Some(U256::from_be_bytes(word))
    }

    pub(crate) fn unsigned_bits(&self) -> usize {
        let mut bit_widths = HashMap::default();
        let mut remaining = usize::MAX;
        self.unsigned_bits_cached(&mut bit_widths, &mut remaining).unwrap_or(256)
    }

    fn unsigned_bits_cached(
        &self,
        bit_widths: &mut HashMap<Self, usize>,
        remaining: &mut usize,
    ) -> Option<usize> {
        if let Some(bits) = bit_widths.get(self) {
            return Some(*bits);
        }

        let mut pending = vec![(self.clone(), false)];
        while let Some((expr, children_visited)) = pending.pop() {
            if bit_widths.contains_key(&expr) {
                continue;
            }
            if !children_visited {
                if *remaining == 0 {
                    return None;
                }
                *remaining -= 1;
                pending.push((expr.clone(), true));
                match expr.kind() {
                    SymExprKind::BinOp(SymBinOp::And, left, right)
                        if right.as_const().is_some() =>
                    {
                        pending.push((left.clone(), false));
                    }
                    SymExprKind::BinOp(SymBinOp::Add | SymBinOp::Mul, left, right)
                    | SymExprKind::Ite(_, left, right) => {
                        pending.push((right.clone(), false));
                        pending.push((left.clone(), false));
                    }
                    SymExprKind::BinOp(SymBinOp::Shl | SymBinOp::Shr, left, right)
                        if right
                            .as_const()
                            .and_then(|shift| usize::try_from(shift).ok())
                            .is_some() =>
                    {
                        pending.push((left.clone(), false));
                    }
                    SymExprKind::BinOp(SymBinOp::UDiv, left, _) => {
                        pending.push((left.clone(), false));
                    }
                    SymExprKind::TernOp(_, _, _, modulus) => {
                        pending.push((modulus.clone(), false));
                    }
                    _ => {}
                }
                continue;
            }

            let bits = match expr.kind() {
                SymExprKind::Const(value) => value.bit_len().max(1),
                SymExprKind::BinOp(SymBinOp::And, left, right) => {
                    if let Some(mask) = right.as_const() {
                        bit_widths[left].min(mask.bit_len())
                    } else {
                        256
                    }
                }
                SymExprKind::BinOp(SymBinOp::Add, left, right) => {
                    bit_widths[left].max(bit_widths[right]).saturating_add(1).min(256)
                }
                SymExprKind::BinOp(SymBinOp::Mul, left, right) => {
                    bit_widths[left].saturating_add(bit_widths[right]).min(256)
                }
                SymExprKind::BinOp(SymBinOp::Shl, left, right) => right
                    .as_const()
                    .and_then(|shift| usize::try_from(shift).ok())
                    .map_or(256, |shift| bit_widths[left].saturating_add(shift).min(256)),
                SymExprKind::BinOp(SymBinOp::Shr, left, right) => right
                    .as_const()
                    .and_then(|shift| usize::try_from(shift).ok())
                    .map_or(256, |shift| bit_widths[left].saturating_sub(shift).max(1)),
                SymExprKind::BinOp(SymBinOp::UDiv, left, _) => bit_widths[left],
                SymExprKind::TernOp(_, _, _, modulus) => bit_widths[modulus],
                SymExprKind::Ite(_, left, right) => bit_widths[left].max(bit_widths[right]),
                _ => 256,
            };
            bit_widths.insert(expr, bits);
        }
        bit_widths.get(self).copied()
    }

    pub(crate) fn extracted_byte(&self, cx: &mut SymCx, index: usize) -> Self {
        debug_assert!(index < 32);
        let shift = Self::constant(cx, U256::from((31 - index) * 8));
        let shifted = Self::binop(cx, SymBinOp::Shr, self.clone(), shift);
        let mask = Self::constant(cx, U256::from(0xff));
        Self::binop(cx, SymBinOp::And, shifted, mask)
    }

    pub(crate) fn extracted_byte_source(&self, index: usize) -> Option<Self> {
        let expr = self.strip_low_byte_mask();
        if index == 31 {
            return Some(expr.clone());
        }
        let SymExprKind::BinOp(SymBinOp::Shr, source, shift) = expr.kind() else { return None };
        let shift = shift.as_const()?;
        (shift == U256::from((31 - index) * 8)).then(|| source.clone())
    }

    pub(crate) fn strip_low_byte_mask(&self) -> &Self {
        match self.kind() {
            SymExprKind::BinOp(SymBinOp::And, left, right)
                if right.as_const() == Some(U256::from(0xff)) =>
            {
                left.strip_low_byte_mask()
            }
            _ => self,
        }
    }

    pub(crate) fn byte_term(&self, cx: &mut SymCx, index: usize) -> Option<Self> {
        debug_assert!(index < 32);

        match self.kind() {
            SymExprKind::Const(value) => {
                Some(Self::constant(cx, U256::from(value.to_be_bytes::<32>()[index])))
            }
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => Some(self.extracted_byte(cx, index)),
            SymExprKind::Not(value) => {
                let value = value.byte_term(cx, index)?;
                Some(Self::not(cx, value))
            }
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                let then_expr = then_expr.byte_term(cx, index)?;
                let else_expr = else_expr.byte_term(cx, index)?;
                Some(Self::ite(cx, cond.clone(), then_expr, else_expr))
            }
            SymExprKind::BinOp(op, left, right) => match op {
                SymBinOp::And => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymBinOp::And,
                    |byte| byte == 0xff,
                    |byte| byte == 0,
                ),
                SymBinOp::Or => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymBinOp::Or,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymBinOp::Xor => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymBinOp::Xor,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymBinOp::Shl => {
                    let shift = right.eval()?;
                    if shift >= U256::from(256) {
                        return Some(Self::zero(cx));
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let source_index = index + shift / 8;
                    if source_index >= 32 {
                        Some(Self::zero(cx))
                    } else {
                        left.byte_term(cx, source_index)
                    }
                }
                SymBinOp::Shr => {
                    let shift = right.eval()?;
                    if shift >= U256::from(256) {
                        return Some(Self::zero(cx));
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let byte_shift = shift / 8;
                    if index < byte_shift {
                        Some(Self::zero(cx))
                    } else {
                        left.byte_term(cx, index - byte_shift)
                    }
                }
                SymBinOp::Add
                | SymBinOp::Sub
                | SymBinOp::Mul
                | SymBinOp::UDiv
                | SymBinOp::URem
                | SymBinOp::SDiv
                | SymBinOp::SRem
                | SymBinOp::Sar => None,
            },
            SymExprKind::TernOp(_, _, _, _) => None,
        }
    }

    fn binary_byte_term(
        cx: &mut SymCx,
        left: &Self,
        right: &Self,
        index: usize,
        op: SymBinOp,
        identity: impl Fn(u8) -> bool,
        absorbing: impl Fn(u8) -> bool,
    ) -> Option<Self> {
        let left = left.byte_term(cx, index)?;
        let right = right.byte_term(cx, index)?;
        match (left.byte_const(), right.byte_const()) {
            (Some(left), _) if absorbing(left) => Some(Self::constant(cx, U256::from(left))),
            (_, Some(right)) if absorbing(right) => Some(Self::constant(cx, U256::from(right))),
            (Some(left), _) if identity(left) => Some(right),
            (_, Some(right)) if identity(right) => Some(left),
            _ => Some(Self::binop(cx, op, left, right)),
        }
    }

    pub(crate) fn byte_const(&self) -> Option<u8> {
        self.as_const().map(|value| value.to::<u8>())
    }

    pub(crate) fn equality_forces_const(
        &self,
        value: U256,
        expr: &Self,
        context: &[SymBoolExpr],
    ) -> Option<U256> {
        if self == expr {
            return Some(value);
        }
        self.equality_forces_const_inner(value, expr, context)
    }

    fn equality_forces_const_inner(
        &self,
        value: U256,
        expr: &Self,
        context: &[SymBoolExpr],
    ) -> Option<U256> {
        let mask = masked_expr_matches(self.kind(), expr)?;
        if value & !mask != U256::ZERO || !context_forces_masked_expr(context, expr, mask) {
            return None;
        }
        Some(value)
    }

    pub(crate) fn nonzero_forces_const(
        &self,
        target: &Self,
        context: &[SymBoolExpr],
    ) -> Option<U256> {
        match self.kind() {
            SymExprKind::Const(_)
            | SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. }
            | SymExprKind::Not(_) => None,
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                if then_expr.eval().is_some_and(|value| !value.is_zero())
                    && else_expr.eval().is_some_and(|value| value.is_zero())
                {
                    cond.forces_expr_const_with_context(target, context)
                } else {
                    None
                }
            }
            SymExprKind::BinOp(SymBinOp::Or, left, right) => {
                if left.eval().is_some_and(|value| value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::BinOp(SymBinOp::And, left, right) => {
                if left.eval().is_some_and(|value| !value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| !value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::BinOp(SymBinOp::Shl | SymBinOp::Shr, value, shift)
                if shift.eval().is_some_and(|shift| shift.is_zero()) =>
            {
                value.nonzero_forces_const(target, context)
            }
            SymExprKind::TernOp(_, _, _, _) => None,
            SymExprKind::BinOp(_, _, _) => None,
        }
    }

    pub(crate) fn is_raw_gasleft(&self) -> bool {
        matches!(self.kind(), SymExprKind::GasLeft(_))
    }

    pub(crate) fn add_const(cx: &mut SymCx, expr: Self, value: U256) -> Self {
        if value.is_zero() {
            return expr;
        }
        match expr.kind() {
            SymExprKind::Const(expr) => Self::constant(cx, expr.wrapping_add(value)),
            _ => {
                let value = Self::constant(cx, value);
                Self::binop(cx, SymBinOp::Add, expr, value)
            }
        }
    }

    /// Visits this expression and all child expressions.
    pub(crate) fn visit<B>(
        &self,
        visitor: &mut impl FnMut(&Self) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        visitor(self)?;
        match self.kind() {
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_) => {}
            SymExprKind::Keccak { len, bytes, .. } => {
                len.visit(visitor)?;
                for byte in bytes.iter() {
                    byte.visit(visitor)?;
                }
            }
            SymExprKind::Hash { bytes, .. } => {
                for byte in bytes.iter() {
                    byte.visit(visitor)?;
                }
            }
            SymExprKind::Not(value) => value.visit(visitor)?,
            SymExprKind::BinOp(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
            SymExprKind::TernOp(_, left, right, modulus) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
                modulus.visit(visitor)?;
            }
            SymExprKind::Ite(cond, left, right) => {
                cond.visit_exprs(visitor)?;
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn visit_bool(&self, mut visitor: impl FnMut(&Self) -> bool) -> bool {
        self.visit(&mut |expr| {
            if visitor(expr) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        })
        .is_break()
    }

    pub(crate) fn fold(
        self,
        cx: &mut SymCx,
        folder: &mut impl FnMut(&mut SymCx, Self) -> Self,
    ) -> Self {
        if matches!(
            self.kind(),
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_)
        ) {
            return folder(cx, self);
        }

        let expr = match self.into_kind() {
            SymExprKind::Keccak { name, len, bytes } => {
                let len = len.fold(cx, folder);
                let bytes = bytes.iter().cloned().map(|byte| byte.fold(cx, folder)).collect();
                Self::keccak_symbol(cx, name, len, bytes)
            }
            SymExprKind::Hash { name, algorithm, bytes } => {
                let bytes = bytes.iter().cloned().map(|byte| byte.fold(cx, folder)).collect();
                Self::hash_symbol(cx, name, algorithm, bytes)
            }
            SymExprKind::Not(value) => {
                let value = value.fold(cx, folder);
                Self::not(cx, value)
            }
            SymExprKind::BinOp(op, left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                Self::binop(cx, op, left, right)
            }
            SymExprKind::TernOp(op, left, right, modulus) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                let modulus = modulus.fold(cx, folder);
                Self::ternop(cx, op, left, right, modulus)
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                let condition = condition.fold_exprs(cx, folder);
                let then_expr = then_expr.fold(cx, folder);
                let else_expr = else_expr.fold(cx, folder);
                Self::ite(cx, condition, then_expr, else_expr)
            }
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_) => {
                unreachable!("leaf expression returned before folding children")
            }
        };
        folder(cx, expr)
    }

    #[cfg(test)]
    pub(crate) fn smt(&self, cx: &SymCx) -> String {
        let mut smt = String::new();
        self.write_smt(cx, &mut smt);
        smt
    }

    pub(in crate::runtime::expr) fn write_smt(&self, cx: &SymCx, out: &mut String) {
        match self.kind() {
            SymExprKind::Const(value) => {
                let _ = write!(out, "(_ bv{value} 256)");
            }
            SymExprKind::Var(symbol)
            | SymExprKind::GasLeft(symbol)
            | SymExprKind::Keccak { name: symbol, .. }
            | SymExprKind::Hash { name: symbol, .. } => out.push_str(cx.symbol_name(*symbol)),
            SymExprKind::Not(value) => {
                out.push_str("(bvnot ");
                value.write_smt(cx, out);
                out.push(')');
            }
            SymExprKind::BinOp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(cx, out);
                out.push(' ');
                right.write_smt(cx, out);
                out.push(')');
            }
            SymExprKind::TernOp(op, left, right, modulus) => {
                write_smt_wide_modular_arithmetic(cx, out, op.smt(), left, right, modulus);
            }
            SymExprKind::Ite(cond, left, right) => {
                out.push_str("(ite ");
                cond.write_smt(cx, out);
                out.push(' ');
                left.write_smt(cx, out);
                out.push(' ');
                right.write_smt(cx, out);
                out.push(')');
            }
        }
    }
}

// Branchless expression rewrites are optional. Bound both the distinct DAG nodes inspected while
// deciding whether to rewrite and the occurrences a later non-memoized solver fold could visit.
const MAX_BRANCHLESS_REWRITE_NODES: usize = 256;
const MAX_BRANCHLESS_REWRITE_UNFOLDED_NODES: usize = 8 * 1024;

struct UnfoldedNodeCounter {
    expr_nodes: HashMap<SymExpr, usize>,
    bool_nodes: HashMap<SymBoolExpr, usize>,
    remaining_unique_nodes: usize,
}

impl UnfoldedNodeCounter {
    fn new() -> Self {
        Self {
            expr_nodes: HashMap::default(),
            bool_nodes: HashMap::default(),
            remaining_unique_nodes: MAX_BRANCHLESS_REWRITE_NODES,
        }
    }

    /// Counts unfolded occurrences while memoizing the cost of each distinct expression node.
    /// Reusing a cached cost still adds every occurrence, which exposes exponential shared DAGs.
    fn expr_nodes(&mut self, expr: &SymExpr) -> Option<usize> {
        if let Some(nodes) = self.expr_nodes.get(expr) {
            return Some(*nodes);
        }
        if self.remaining_unique_nodes == 0 {
            return None;
        }
        self.remaining_unique_nodes -= 1;

        let nodes = match expr.kind() {
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_) => 1,
            SymExprKind::Keccak { len, bytes, .. } => {
                let mut nodes = 1usize.checked_add(self.expr_nodes(len)?)?;
                for byte in bytes.iter() {
                    nodes = nodes.checked_add(self.expr_nodes(byte)?)?;
                }
                nodes
            }
            SymExprKind::Hash { bytes, .. } => {
                let mut nodes = 1usize;
                for byte in bytes.iter() {
                    nodes = nodes.checked_add(self.expr_nodes(byte)?)?;
                }
                nodes
            }
            SymExprKind::Not(value) => 1usize.checked_add(self.expr_nodes(value)?)?,
            SymExprKind::BinOp(_, left, right) => {
                1usize.checked_add(self.expr_nodes(left)?)?.checked_add(self.expr_nodes(right)?)?
            }
            SymExprKind::TernOp(_, left, right, modulus) => 1usize
                .checked_add(self.expr_nodes(left)?)?
                .checked_add(self.expr_nodes(right)?)?
                .checked_add(self.expr_nodes(modulus)?)?,
            SymExprKind::Ite(condition, then_expr, else_expr) => 1usize
                .checked_add(self.bool_nodes(condition)?)?
                .checked_add(self.expr_nodes(then_expr)?)?
                .checked_add(self.expr_nodes(else_expr)?)?,
        };
        if nodes > MAX_BRANCHLESS_REWRITE_UNFOLDED_NODES {
            return None;
        }
        self.expr_nodes.insert(expr.clone(), nodes);
        Some(nodes)
    }

    fn bool_nodes(&mut self, expr: &SymBoolExpr) -> Option<usize> {
        if let Some(nodes) = self.bool_nodes.get(expr) {
            return Some(*nodes);
        }
        if self.remaining_unique_nodes == 0 {
            return None;
        }
        self.remaining_unique_nodes -= 1;

        let nodes = match expr.kind() {
            SymBoolExprKind::Const(_) => 1,
            SymBoolExprKind::Not(value) => 1usize.checked_add(self.bool_nodes(value)?)?,
            SymBoolExprKind::And(values) => {
                let mut nodes = 1usize;
                for value in values.iter() {
                    nodes = nodes.checked_add(self.bool_nodes(value)?)?;
                }
                nodes
            }
            SymBoolExprKind::Cmp(_, left, right) => {
                1usize.checked_add(self.expr_nodes(left)?)?.checked_add(self.expr_nodes(right)?)?
            }
        };
        if nodes > MAX_BRANCHLESS_REWRITE_UNFOLDED_NODES {
            return None;
        }
        self.bool_nodes.insert(expr.clone(), nodes);
        Some(nodes)
    }
}

fn write_smt_wide_modular_arithmetic(
    cx: &SymCx,
    out: &mut String,
    op: &'static str,
    left: &SymExpr,
    right: &SymExpr,
    modulus: &SymExpr,
) {
    // if modulus == 0:
    //   0
    // else:
    //   low_256((zext(left) op zext(right)) urem zext(modulus))
    out.push_str("(ite (= ");
    modulus.write_smt(cx, out);
    out.push_str(" (_ bv0 256)) (_ bv0 256) ((_ extract 255 0) (bvurem (");
    out.push_str(op);
    out.push_str(" ((_ zero_extend 256) ");
    left.write_smt(cx, out);
    out.push_str(") ((_ zero_extend 256) ");
    right.write_smt(cx, out);
    out.push_str(")) ((_ zero_extend 256) ");
    modulus.write_smt(cx, out);
    out.push_str("))))");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SymTernOp {
    AddMod,
    MulMod,
}

impl SymTernOp {
    pub(crate) const fn smt(self) -> &'static str {
        match self {
            Self::AddMod => "bvadd",
            Self::MulMod => "bvmul",
        }
    }

    pub(crate) fn eval(self, left: U256, right: U256, modulus: U256) -> U256 {
        if modulus.is_zero() {
            return U256::ZERO;
        }
        match self {
            Self::AddMod => left.add_mod(right, modulus),
            Self::MulMod => left.mul_mod(right, modulus),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SymBinOp {
    Add,
    Sub,
    Mul,
    UDiv,
    URem,
    SDiv,
    SRem,
    And,
    Or,
    Xor,
    Shl,
    Shr,
    Sar,
}

impl SymBinOp {
    pub(crate) const fn smt(self) -> &'static str {
        match self {
            Self::Add => "bvadd",
            Self::Sub => "bvsub",
            Self::Mul => "bvmul",
            Self::UDiv => "bvudiv",
            Self::URem => "bvurem",
            Self::SDiv => "bvsdiv",
            Self::SRem => "bvsrem",
            Self::And => "bvand",
            Self::Or => "bvor",
            Self::Xor => "bvxor",
            Self::Shl => "bvshl",
            Self::Shr => "bvlshr",
            Self::Sar => "bvashr",
        }
    }

    pub(crate) fn eval(self, left: U256, right: U256) -> U256 {
        match self {
            Self::Add => left.wrapping_add(right),
            Self::Sub => left.wrapping_sub(right),
            Self::Mul => left.wrapping_mul(right),
            Self::UDiv => {
                if right.is_zero() {
                    U256::ZERO
                } else {
                    left / right
                }
            }
            Self::URem => {
                if right.is_zero() {
                    U256::ZERO
                } else {
                    left % right
                }
            }
            Self::SDiv => sdiv(left, right),
            Self::SRem => smod(left, right),
            Self::And => left & right,
            Self::Or => left | right,
            Self::Xor => left ^ right,
            Self::Shl => {
                if right >= U256::from(256) {
                    U256::ZERO
                } else {
                    left << usize::try_from(right).expect("checked word shift")
                }
            }
            Self::Shr => {
                if right >= U256::from(256) {
                    U256::ZERO
                } else {
                    left >> usize::try_from(right).expect("checked word shift")
                }
            }
            Self::Sar => {
                if right >= U256::from(256) {
                    sar(left, 256)
                } else {
                    sar(left, usize::try_from(right).expect("checked word shift"))
                }
            }
        }
    }
}

pub(crate) fn keccak_word(cx: &mut SymCx, bytes: Vec<SymExpr>) -> SymExpr {
    let len = bytes.len();
    let len = SymExpr::constant(cx, U256::from(len));
    keccak_word_with_len(cx, bytes, len)
}

pub(crate) fn keccak_word_with_len(cx: &mut SymCx, bytes: Vec<SymExpr>, len: SymExpr) -> SymExpr {
    if let Some(len) = len.as_const()
        && let Ok(len) = usize::try_from(len)
        && len <= bytes.len()
        && let Ok(concrete) = concrete_expr_bytes(&bytes[..len], "symbolic keccak input")
    {
        let hash = U256::from_be_bytes(keccak256(concrete).0);
        if len == 64 {
            cx.record_concrete_keccak_preimage(hash, bytes[..len].to_vec().into());
        }
        return SymExpr::constant(cx, hash);
    }

    let exprs = bytes;
    let name = stable_symbol(cx, "keccak", format!("{len:?}:{exprs:?}").as_bytes());
    SymExpr::keccak_symbol(cx, name, len, exprs)
}

pub(crate) fn symbolic_hash_word_with_len(
    cx: &mut SymCx,
    algorithm: &'static str,
    bytes: Vec<SymExpr>,
    len: SymExpr,
) -> SymExpr {
    let exprs = bytes;
    let name = stable_symbol(cx, algorithm, format!("{len:?}:{exprs:?}").as_bytes());
    let mut identity = Vec::with_capacity(exprs.len() + 1);
    identity.push(len);
    identity.extend(exprs);
    SymExpr::hash_symbol(cx, name, algorithm, identity)
}

pub(crate) fn create2_address_word(
    cx: &mut SymCx,
    state: &mut PathState,
    creator: Address,
    salt: SymExpr,
    initcode: &SymCode,
) -> Result<(SymExpr, Address), SymbolicError> {
    match (salt.as_const(), initcode.concrete_bytes(cx, "symbolic CREATE2 initcode")) {
        (Some(salt), Ok(initcode)) => {
            let address = creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode);
            Ok((SymExpr::constant(cx, address_word(address)), address))
        }
        (None, Ok(initcode)) => {
            let initcode_hash = keccak256(&initcode);
            let word = symbolic_create2_address_word(
                cx,
                state,
                format!("{creator:?}"),
                salt,
                format!("{initcode_hash:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (_, Err(SymbolicError::Unsupported("symbolic CREATE2 initcode"))) => {
            let initcode_bytes = initcode.read_byte_exprs(cx, 0, initcode.len());
            let word = symbolic_create2_address_word(
                cx,
                state,
                format!("{creator:?}"),
                salt,
                format!("{initcode_bytes:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (_, Err(err)) => Err(err),
    }
}

pub(crate) fn compute_create2_address_word(
    cx: &mut SymCx,
    state: &mut PathState,
    deployer: SymExpr,
    salt: SymExpr,
    init_code_hash: SymExpr,
) -> Result<SymExpr, SymbolicError> {
    let deployer_concrete = state.constrained_word(cx, &deployer).map(word_to_address);
    let salt_concrete = state.constrained_word(cx, &salt);
    let init_code_hash_concrete = state.constrained_word(cx, &init_code_hash);

    if let (Some(deployer), Some(salt), Some(init_code_hash)) =
        (deployer_concrete, salt_concrete, init_code_hash_concrete)
    {
        let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
        let address = deployer.create2(B256::from(salt.to_be_bytes::<32>()), init_code_hash);
        return Ok(SymExpr::constant(cx, address_word(address)));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{deployer:?}"));
    let init_code_hash_identity = init_code_hash_concrete
        .map(|init_code_hash| {
            let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
            format!("{init_code_hash:?}")
        })
        .unwrap_or_else(|| format!("{init_code_hash:?}"));

    Ok(symbolic_create2_address_word(cx, state, deployer_identity, salt, init_code_hash_identity))
}

pub(crate) fn compute_create_address_word(
    cx: &mut SymCx,
    state: &mut PathState,
    deployer: SymExpr,
    nonce: SymExpr,
) -> Result<SymExpr, SymbolicError> {
    let deployer_concrete = state.constrained_word(cx, &deployer).map(word_to_address);
    let nonce_concrete = state.constrained_word(cx, &nonce);

    if let (Some(deployer), Some(nonce)) = (deployer_concrete, nonce_concrete) {
        let Ok(nonce) = u64::try_from(nonce) else {
            return Err(SymbolicError::Unsupported("symbolic vm.computeCreateAddress nonce"));
        };
        return Ok(SymExpr::constant(cx, address_word(deployer.create(nonce))));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{deployer:?}"));
    Ok(symbolic_create_address_word(cx, state, deployer_identity, nonce))
}

pub(crate) fn symbolic_create_address_word(
    cx: &mut SymCx,
    state: &mut PathState,
    creator_identity: String,
    nonce: SymExpr,
) -> SymExpr {
    let name =
        stable_symbol(cx, "create_address", format!("{creator_identity}:{nonce:?}").as_bytes());
    let word = SymExpr::get_var(cx, name);
    state.constraints.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ult, &word, U256::ONE << 160));
    word
}

pub(crate) fn symbolic_create2_address_word(
    cx: &mut SymCx,
    state: &mut PathState,
    creator_identity: String,
    salt: SymExpr,
    initcode_identity: String,
) -> SymExpr {
    let name = stable_symbol(
        cx,
        "create2_address",
        format!("{creator_identity}:{salt:?}:{initcode_identity}").as_bytes(),
    );
    let word = SymExpr::get_var(cx, name);
    state.constraints.push(SymBoolExpr::cmp_word_const(cx, SymCmpOp::Ult, &word, U256::ONE << 160));
    word
}

#[cfg(test)]
mod tests {
    use super::*;

    fn indexed_bool_word(cx: &mut SymCx, source: &SymExpr, index: usize) -> SymExpr {
        let value = SymExpr::constant(cx, U256::from(index));
        let condition = SymBoolExpr::eq(cx, source.clone(), value);
        SymExpr::bool_word(cx, condition)
    }

    #[test]
    fn bitwise_bool_word_condition_visits_shared_or_dag_once() {
        let mut cx = SymCx::new();
        let source = SymExpr::var(&mut cx, "source");
        let mut word = indexed_bool_word(&mut cx, &source, 0);
        for index in 1..=26 {
            let next_word = indexed_bool_word(&mut cx, &source, index);
            let nested_word = SymExpr::binop(&mut cx, SymBinOp::Or, word.clone(), next_word);
            word = SymExpr::binop(&mut cx, SymBinOp::Or, word, nested_word);
        }

        assert!(word.bitwise_bool_word_condition(&mut cx).is_some());
    }

    #[test]
    fn bitwise_bool_word_condition_stops_at_shared_visit_budget() {
        let mut cx = SymCx::new();
        let source = SymExpr::var(&mut cx, "source");
        let mut word = indexed_bool_word(&mut cx, &source, 0);
        for index in 1..=MAX_BITWISE_BOOL_WORD_VISITS {
            let next_word = indexed_bool_word(&mut cx, &source, index);
            word = SymExpr::binop(&mut cx, SymBinOp::Or, word, next_word);
        }

        assert!(word.bitwise_bool_word_condition(&mut cx).is_none());
        let one = SymExpr::one(&mut cx);
        let mask = SymExpr::binop(&mut cx, SymBinOp::Sub, word, one);
        assert!(matches!(mask.kind(), SymExprKind::BinOp(SymBinOp::Sub, _, _)));
    }

    #[test]
    fn bitwise_bool_word_condition_deduplicates_shared_or_dag() {
        let mut cx = SymCx::new();
        let base = SymExpr::var(&mut cx, "base");
        let selected = SymExpr::var(&mut cx, "selected");
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, y);
        let mut condition_word = SymExpr::bool_word(&mut cx, condition);
        for _ in 0..64 {
            condition_word = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::Or, condition_word.clone(), condition_word.clone()),
            );
        }
        let delta = SymExpr::binop(&mut cx, SymBinOp::Xor, base.clone(), selected.clone());
        let selector = SymExpr::binop(&mut cx, SymBinOp::Mul, condition_word, delta);
        let actual = SymExpr::binop(&mut cx, SymBinOp::Xor, base.clone(), selector);

        let SymExprKind::Ite(_, then_expr, else_expr) = actual.kind() else {
            panic!("shared boolean selector was not recovered");
        };
        assert_eq!(then_expr, &selected);
        assert_eq!(else_expr, &base);
    }

    #[test]
    fn bitwise_bool_word_condition_deduplicates_overlapping_or_dag() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let first = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), y.clone());
        let second = SymBoolExpr::cmp(&mut cx, SymCmpOp::Eq, x, y);
        let mut previous = SymExpr::bool_word(&mut cx, first.clone());
        let mut current = SymExpr::bool_word(&mut cx, second.clone());
        for _ in 0..28 {
            let next = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::Or, current.clone(), previous.clone()),
            );
            previous = current;
            current = next;
        }

        let actual = current.bitwise_bool_word_condition(&mut cx).expect("boolean condition");
        let expected = SymBoolExpr::or(&mut cx, vec![second, first]);

        assert_eq!(actual, expected);
    }

    #[test]
    fn bitwise_bool_word_condition_stops_at_node_budget() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, y);
        let bool_word = SymExpr::bool_word(&mut cx, condition);
        let mut condition_word = bool_word.clone();
        for _ in 0..MAX_BITWISE_BOOL_WORD_VISITS {
            condition_word = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::Or, condition_word, bool_word.clone()),
            );
        }

        assert!(condition_word.bitwise_bool_word_condition(&mut cx).is_none());
    }

    #[test]
    fn commutative_branchless_rewrites_produce_canonical_ites() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let first_condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), y.clone());
        let second_condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Eq, x, y);

        let one = SymExpr::one(&mut cx);
        let two = SymExpr::constant(&mut cx, U256::from(2));
        let three = SymExpr::constant(&mut cx, U256::from(3));
        let four = SymExpr::constant(&mut cx, U256::from(4));
        let first_offset = SymExpr::ite(&mut cx, first_condition.clone(), one, two);
        let second_offset = SymExpr::ite(&mut cx, second_condition.clone(), three, four);
        let add_forward =
            SymExpr::binop(&mut cx, SymBinOp::Add, first_offset.clone(), second_offset.clone());
        let add_reverse = SymExpr::binop(&mut cx, SymBinOp::Add, second_offset, first_offset);
        assert_eq!(add_forward, add_reverse);
        let SymExprKind::Ite(_, then_expr, else_expr) = add_forward.kind() else {
            panic!("dual ITE addition did not rewrite");
        };
        assert!(matches!(then_expr.kind(), SymExprKind::BinOp(SymBinOp::Add, _, _)));
        assert!(matches!(else_expr.kind(), SymExprKind::BinOp(SymBinOp::Add, _, _)));

        let first_word = SymExpr::bool_word(&mut cx, first_condition.clone());
        let second_word = SymExpr::bool_word(&mut cx, second_condition.clone());
        let mul_forward =
            SymExpr::binop(&mut cx, SymBinOp::Mul, first_word.clone(), second_word.clone());
        let mul_reverse = SymExpr::binop(&mut cx, SymBinOp::Mul, second_word, first_word);
        assert_eq!(mul_forward, mul_reverse);
        let SymExprKind::Ite(_, then_expr, else_expr) = mul_forward.kind() else {
            panic!("dual boolean-word multiplication did not rewrite");
        };
        assert!(matches!(then_expr.kind(), SymExprKind::Ite(..)));
        assert!(else_expr.as_const().is_some_and(|value| value.is_zero()));

        let zero = SymExpr::zero(&mut cx);
        let first_value = SymExpr::var(&mut cx, "first_value");
        let second_value = SymExpr::var(&mut cx, "second_value");
        let first_selected = SymExpr::ite(&mut cx, first_condition, first_value, zero.clone());
        let second_selected = SymExpr::ite(&mut cx, second_condition, second_value, zero);
        let xor_forward =
            SymExpr::binop(&mut cx, SymBinOp::Xor, first_selected.clone(), second_selected.clone());
        let xor_reverse = SymExpr::binop(&mut cx, SymBinOp::Xor, second_selected, first_selected);
        assert_eq!(xor_forward, xor_reverse);
        let SymExprKind::Ite(_, then_expr, else_expr) = xor_forward.kind() else {
            panic!("dual zero-ITE XOR did not rewrite");
        };
        assert!(matches!(then_expr.kind(), SymExprKind::Ite(..)));
        assert!(matches!(else_expr.kind(), SymExprKind::Ite(..)));
    }

    #[test]
    fn addition_keeps_exponentially_shared_ite_operand_raw() {
        let mut cx = SymCx::new();
        let mut value = SymExpr::var(&mut cx, "value");
        for index in 0..32 {
            let selector = SymExpr::var(&mut cx, &format!("add_selector_{index}"));
            let condition = SymBoolExpr::eq_word_const(&mut cx, &selector, U256::ZERO);
            let then_value = SymExpr::constant(&mut cx, U256::from(2 * index + 2));
            let else_value = SymExpr::constant(&mut cx, U256::from(2 * index + 3));
            let offset = SymExpr::ite(&mut cx, condition, then_value, else_value);
            value = SymExpr::binop(&mut cx, SymBinOp::Add, value, offset);
        }

        assert!(matches!(value.kind(), SymExprKind::BinOp(SymBinOp::Add, _, _)));
    }

    #[test]
    fn xor_keeps_exponentially_shared_ite_operand_raw() {
        let mut cx = SymCx::new();
        let zero = SymExpr::zero(&mut cx);
        let mut value = SymExpr::var(&mut cx, "value");
        for index in 0..32 {
            let selector = SymExpr::var(&mut cx, &format!("xor_selector_{index}"));
            let condition = SymBoolExpr::eq_word_const(&mut cx, &selector, U256::ZERO);
            let selected = SymExpr::var(&mut cx, &format!("xor_selected_{index}"));
            let conditional = SymExpr::ite(&mut cx, condition, selected, zero.clone());
            value = SymExpr::binop(&mut cx, SymBinOp::Xor, value, conditional);
        }

        assert!(matches!(value.kind(), SymExprKind::BinOp(SymBinOp::Xor, _, _)));
    }

    #[test]
    fn bitwise_bool_word_condition_bounds_bit_width_analysis() {
        let mut cx = SymCx::new();
        let one = SymExpr::one(&mut cx);
        let mut expression = one.clone();
        for _ in 0..MAX_BITWISE_BOOL_WORD_VISITS {
            expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::UDiv, expression, one.clone()),
            );
        }

        assert!(expression.bitwise_bool_word_condition(&mut cx).is_none());
    }

    #[test]
    fn bitwise_bool_word_condition_keeps_one_bit_leaf_comparison_raw() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let first = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x.clone(), y.clone());
        let second = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ugt, x, y);
        let zero = SymExpr::zero(&mut cx);
        let one = SymExpr::one(&mut cx);
        let nested = SymExpr::from_kind(&mut cx, SymExprKind::Ite(second, zero.clone(), one));
        let leaf = SymExpr::from_kind(&mut cx, SymExprKind::Ite(first, nested, zero.clone()));

        let actual =
            leaf.bitwise_bool_word_condition(&mut cx).expect("one-bit leaf should be recovered");
        let (leaf, zero) = SymExpr::ordered_commutative_operands(leaf, zero);
        let raw_zero_check =
            SymBoolExpr::from_kind(&mut cx, SymBoolExprKind::Cmp(SymCmpOp::Eq, leaf, zero));
        let expected = raw_zero_check.not(&mut cx);

        assert_eq!(actual, expected);
    }

    #[test]
    fn unsigned_bit_width_handles_deep_expression_iteratively() {
        let mut cx = SymCx::new();
        let one = SymExpr::one(&mut cx);
        let mut expression = one.clone();
        for _ in 0..2048 {
            expression = SymExpr::from_kind(
                &mut cx,
                SymExprKind::BinOp(SymBinOp::UDiv, expression, one.clone()),
            );
        }

        assert_eq!(expression.unsigned_bits(), 1);
    }

    #[test]
    fn xor_select_rejects_delta_before_recovering_condition() {
        let mut cx = SymCx::new();
        let base = SymExpr::var(&mut cx, "base");
        let unrelated_left = SymExpr::var(&mut cx, "unrelated_left");
        let unrelated_right = SymExpr::var(&mut cx, "unrelated_right");
        let delta = SymExpr::binop(&mut cx, SymBinOp::Xor, unrelated_left, unrelated_right);
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let condition = SymBoolExpr::cmp(&mut cx, SymCmpOp::Ult, x, y);
        let condition_word = SymExpr::bool_word(&mut cx, condition);
        let selector = SymExpr::binop(&mut cx, SymBinOp::Mul, condition_word, delta);

        assert!(SymExpr::xor_with_bool_select(&mut cx, &base, &selector).is_none());
    }

    #[test]
    fn saturating_mul_rewrite_preserves_boundary_values() {
        let mut cx = SymCx::new();
        let x = SymExpr::var(&mut cx, "x");
        let y = SymExpr::var(&mut cx, "y");
        let x_symbol = match x.kind() {
            SymExprKind::Var(symbol) => *symbol,
            _ => unreachable!("constructed symbolic variable"),
        };
        let y_symbol = match y.kind() {
            SymExprKind::Var(symbol) => *symbol,
            _ => unreachable!("constructed symbolic variable"),
        };

        let zero = SymExpr::zero(&mut cx);
        let x_is_zero = SymBoolExpr::eq(&mut cx, x.clone(), zero);
        let product = SymExpr::binop(&mut cx, SymBinOp::Mul, x.clone(), y.clone());
        let quotient = SymExpr::binop(&mut cx, SymBinOp::UDiv, product.clone(), x);
        let product_is_exact = SymBoolExpr::eq(&mut cx, quotient, y);
        let safe = SymBoolExpr::or(&mut cx, vec![product_is_exact.clone(), x_is_zero.clone()]);
        let x_is_zero_word = SymExpr::bool_word(&mut cx, x_is_zero);
        let product_is_exact_word = SymExpr::bool_word(&mut cx, product_is_exact);
        let guard = SymExpr::binop(&mut cx, SymBinOp::Or, x_is_zero_word, product_is_exact_word);
        let one = SymExpr::one(&mut cx);
        let raw_mask =
            SymExpr::from_kind(&mut cx, SymExprKind::BinOp(SymBinOp::Sub, guard.clone(), one));
        let original = SymExpr::from_kind(
            &mut cx,
            SymExprKind::BinOp(SymBinOp::Or, raw_mask, product.clone()),
        );

        let one = SymExpr::one(&mut cx);
        let simplified_mask = SymExpr::binop(&mut cx, SymBinOp::Sub, guard, one);
        let simplified = SymExpr::binop(&mut cx, SymBinOp::Or, simplified_mask, product.clone());
        let max = SymExpr::constant(&mut cx, U256::MAX);
        let expected = SymExpr::ite(&mut cx, safe, product, max);
        assert_eq!(simplified, expected);

        let half_range = U256::ONE << 255;
        let boundaries = [
            (U256::ZERO, U256::MAX),
            (U256::MAX, U256::ZERO),
            (U256::MAX, U256::ONE),
            (U256::ONE, U256::MAX),
            (U256::MAX, U256::from(2)),
            (U256::from(2), U256::MAX),
            (half_range, U256::from(2)),
            (U256::from(2), half_range),
        ];
        for (x_value, y_value) in boundaries {
            let mut model = SymbolicModel::default();
            model.insert(x_symbol, x_value);
            model.insert(y_symbol, y_value);
            let expected_value = x_value.checked_mul(y_value).unwrap_or(U256::MAX);
            assert_eq!(original.eval_model(&model).unwrap(), expected_value);
            assert_eq!(simplified.eval_model(&model).unwrap(), expected_value);
        }
    }
}
