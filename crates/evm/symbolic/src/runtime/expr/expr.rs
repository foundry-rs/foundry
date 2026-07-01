use super::{hashcons::HashConsed, *};

pub(crate) fn keccak_word(cx: &mut SymCx, bytes: Vec<SymExpr>) -> SymExpr {
    let len = bytes.len();
    let len = SymExpr::constant(cx, U256::from(len));
    keccak_word_with_len(cx, bytes, len)
}

pub(crate) fn keccak_word_with_len(cx: &mut SymCx, bytes: Vec<SymExpr>, len: SymExpr) -> SymExpr {
    if let Some(len) = len.as_const()
        && let Ok(len) = usize::try_from(len)
        && len <= bytes.len()
        && let Ok(bytes) = concrete_expr_bytes(&bytes[..len], "symbolic keccak input")
    {
        return SymExpr::constant(cx, U256::from_be_bytes(keccak256(bytes).0));
    }

    let exprs = bytes;
    let name = stable_symbol("keccak", format!("{len:?}:{exprs:?}").as_bytes());
    SymExpr::keccak_symbol(cx, name, len, exprs)
}

pub(crate) fn symbolic_hash_word_with_len(
    cx: &mut SymCx,
    algorithm: &'static str,
    bytes: Vec<SymExpr>,
    len: SymExpr,
) -> SymExpr {
    let exprs = bytes;
    let name = stable_symbol(algorithm, format!("{len:?}:{exprs:?}").as_bytes());
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
    let name = stable_symbol("create_address", format!("{creator_identity}:{nonce:?}").as_bytes());
    let word = SymExpr::var_symbol(cx, name);
    state.constraints.push(SymBoolExpr::cmp_word_const(
        cx,
        SymCmpOp::Ult,
        &word,
        U256::from(1) << 160,
    ));
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
        "create2_address",
        format!("{creator_identity}:{salt:?}:{initcode_identity}").as_bytes(),
    );
    let word = SymExpr::var_symbol(cx, name);
    state.constraints.push(SymBoolExpr::cmp_word_const(
        cx,
        SymCmpOp::Ult,
        &word,
        U256::from(1) << 160,
    ));
    word
}

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
                let read_base = SymBoolExpr::eq(cx, read_base, write_base);
                let read_offset = SymBoolExpr::eq(cx, read_offset, write_offset);
                SymBoolExpr::and(cx, vec![read_base, read_offset])
            }
            (Some(_), None) if write_key.as_const().is_some() => SymBoolExpr::constant(cx, false),
            (None, Some(_)) if self.as_const().is_some() => SymBoolExpr::constant(cx, false),
            _ => SymBoolExpr::eq(cx, self.clone(), write_key.clone()),
        }
    }

    fn storage_mapping_root_slot(&self, cx: &mut SymCx) -> Option<U256> {
        let SymExprKind::Keccak { len, bytes, .. } = self.kind() else { return None };
        if len.as_const() != Some(U256::from(64)) || bytes.len() < 64 {
            return None;
        }

        let slot = Self::from_bytes(cx, bytes[32..64].iter().cloned());
        match slot.kind() {
            SymExprKind::Const(slot) => Some(*slot),
            SymExprKind::Keccak { .. } => slot.storage_mapping_root_slot(cx),
            _ => None,
        }
    }

    fn storage_layout_key(&self, cx: &mut SymCx) -> Option<(Self, Self)> {
        match self.kind() {
            SymExprKind::Keccak { .. } => Some((self.clone(), Self::zero(cx))),
            SymExprKind::BinOp(SymExprBinOp::Add, left, right) => {
                if let Some((base, offset)) = left.storage_layout_key(cx)
                    && !right.contains_keccak()
                {
                    let offset = Self::binop(cx, SymExprBinOp::Add, offset, right.clone());
                    return Some((base, offset));
                }
                if let Some((base, offset)) = right.storage_layout_key(cx)
                    && !left.contains_keccak()
                {
                    let offset = Self::binop(cx, SymExprBinOp::Add, offset, left.clone());
                    return Some((base, offset));
                }
                None
            }
            _ => None,
        }
    }
}

fn masked_expr_matches(candidate: &SymExprKind, target: &SymExpr) -> Option<U256> {
    match candidate {
        SymExprKind::BinOp(SymExprBinOp::And, left, right) if left == target => right.eval(),
        SymExprKind::BinOp(SymExprBinOp::And, left, right) if right == target => left.eval(),
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
    GasLeft(usize),
    Keccak { name: Symbol, len: SymExpr, bytes: Arc<[SymExpr]> },
    Hash { name: Symbol, algorithm: &'static str, bytes: Arc<[SymExpr]> },
    Not(SymExpr),
    BinOp(SymExprBinOp, SymExpr, SymExpr),
    TernOp(SymExprTernOp, SymExpr, SymExpr, SymExpr),
    Ite(SymBoolExpr, SymExpr, SymExpr),
}

impl SymExpr {
    pub(in crate::runtime) fn kind(&self) -> &SymExprKind {
        self.kind.value()
    }

    #[cfg(test)]
    pub(crate) fn ptr_eq(&self, other: &Self) -> bool {
        self.kind.ptr_eq(&other.kind)
    }

    #[cfg(test)]
    pub(crate) fn var_name(&self) -> Option<&str> {
        match self.kind() {
            SymExprKind::Var(name) => Some(name.as_str()),
            _ => None,
        }
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
        Self::constant(cx, U256::from(1))
    }

    pub(crate) fn constant(cx: &mut SymCx, value: U256) -> Self {
        if value.is_zero() {
            return cx.cached_zero();
        }
        if value == U256::from(1) {
            return cx.cached_one();
        }
        Self::from_kind(cx, SymExprKind::Const(value))
    }

    pub(crate) fn var(cx: &mut SymCx, name: &str) -> Self {
        let symbol = cx.intern(name);
        Self::var_symbol(cx, symbol)
    }

    pub(crate) fn var_symbol(cx: &mut SymCx, name: Symbol) -> Self {
        Self::from_kind(cx, SymExprKind::Var(name))
    }

    pub(crate) fn gas_left(cx: &mut SymCx, id: usize) -> Self {
        Self::from_kind(cx, SymExprKind::GasLeft(id))
    }

    pub(crate) fn not(cx: &mut SymCx, value: Self) -> Self {
        match value.kind() {
            SymExprKind::Const(value) => Self::constant(cx, !*value),
            SymExprKind::Not(value) => value.clone(),
            _ => Self::from_kind(cx, SymExprKind::Not(value)),
        }
    }

    pub(crate) fn binop(cx: &mut SymCx, binop: SymExprBinOp, left: Self, right: Self) -> Self {
        match binop {
            SymExprBinOp::Add => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const + const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `0 + a => a`.
                (SymExprKind::Const(value), _) if value.is_zero() => right,
                // `a + 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                _ => Self::commutative_binop(cx, binop, left, right),
            },
            SymExprBinOp::Sub => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const - const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a - 0 => a`.
                (_, SymExprKind::Const(value)) if value.is_zero() => left,
                // `a - a => 0`.
                _ if left == right => Self::zero(cx),
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymExprBinOp::Mul => match (left.kind(), right.kind()) {
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
                (SymExprKind::Const(value), _) if *value == U256::from(1) => right,
                // `a * 1 => a`.
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => left,
                _ => Self::commutative_binop(cx, binop, left, right),
            },
            SymExprBinOp::UDiv | SymExprBinOp::SDiv => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const / const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a / 0 => 0`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::zero(cx),
                // `a / 1 => a`.
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => left,
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymExprBinOp::URem | SymExprBinOp::SRem => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    // `const % const => const`.
                    Self::constant(cx, binop.eval(*left_value, *right_value))
                }
                // `a % 0 => 0`.
                (_, SymExprKind::Const(value)) if value.is_zero() => Self::zero(cx),
                // `a % 1 => 0`.
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => Self::zero(cx),
                _ => Self::from_kind(cx, SymExprKind::BinOp(binop, left, right)),
            },
            SymExprBinOp::And => match (left.kind(), right.kind()) {
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
            SymExprBinOp::Or => match (left.kind(), right.kind()) {
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
                _ => Self::or(cx, left, right),
            },
            SymExprBinOp::Xor => match (left.kind(), right.kind()) {
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
                _ => Self::commutative_binop(cx, binop, left, right),
            },
            SymExprBinOp::Shl => match (left.kind(), right.kind()) {
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
            SymExprBinOp::Shr => match (left.kind(), right.kind()) {
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
            SymExprBinOp::Sar => match (left.kind(), right.kind()) {
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
        ternop: SymExprTernOp,
        left: Self,
        right: Self,
        modulus: Self,
    ) -> Self {
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus))
                if modulus.is_zero() || *modulus == U256::from(1) =>
            {
                // `addmod/mulmod(a, b, 0) => 0`.
                Self::zero(cx)
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                // `addmod/mulmod(const, const, const) => const`.
                Self::constant(cx, ternop.eval(*left, *right, *modulus))
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
            None if then_expr.as_const() == Some(U256::from(1))
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
        Self::commutative_binop(cx, SymExprBinOp::Or, left, right)
    }

    fn commutative_binop(cx: &mut SymCx, op: SymExprBinOp, left: Self, right: Self) -> Self {
        // `a + b => b + a`.
        let (left, right) = Self::ordered_commutative_operands(left, right);
        Self::from_kind(cx, SymExprKind::BinOp(op, left, right))
    }

    pub(in crate::runtime::expr) fn ordered_commutative_operands(
        left: Self,
        right: Self,
    ) -> (Self, Self) {
        if right.kind.cached_hash() < left.kind.cached_hash() {
            (right, left)
        } else {
            (left, right)
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
            SymExprKind::BinOp(SymExprBinOp::Or, left, right) => {
                // `(a | b) & mask => (a & mask) | (b & mask)`.
                let left = Self::and_const(cx, left.clone(), mask);
                let right = Self::and_const(cx, right.clone(), mask);
                Self::binop(cx, SymExprBinOp::Or, left, right)
            }
            SymExprKind::BinOp(SymExprBinOp::Shl, _, shift)
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
            SymExprKind::BinOp(SymExprBinOp::And, left, right) => match (left.kind(), right.kind())
            {
                (SymExprKind::Const(value), _) if *value == mask => {
                    // `(mask & a) & mask => a & mask`.
                    Self::and_const(cx, right.clone(), mask)
                }
                (_, SymExprKind::Const(value)) if *value == mask => {
                    // `(a & mask) & mask => a & mask`.
                    Self::and_const(cx, left.clone(), mask)
                }
                // `(a & a) & mask => a & mask`.
                _ if left == right => Self::and_const(cx, left.clone(), mask),
                _ => {
                    let mask = Self::constant(cx, mask);
                    Self::from_kind(cx, SymExprKind::BinOp(SymExprBinOp::And, expr, mask))
                }
            },
            _ => {
                let mask = Self::constant(cx, mask);
                Self::from_kind(cx, SymExprKind::BinOp(SymExprBinOp::And, expr, mask))
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

        if let SymExprKind::BinOp(SymExprBinOp::Shl, inner, left_shift) = expr.kind()
            && left_shift.as_const() == Some(U256::from(shift))
            && inner.unsigned_bits() <= 256 - shift
        {
            // `(a << n) >> n => a`.
            return inner.clone();
        }

        if let SymExprKind::BinOp(SymExprBinOp::Or, left, right) = expr.kind() {
            // `(a | b) >> n => (a >> n) | (b >> n)`.
            let left = Self::shr_const(cx, left.clone(), U256::from(shift));
            let right = Self::shr_const(cx, right.clone(), U256::from(shift));
            return Self::binop(cx, SymExprBinOp::Or, left, right);
        }

        let shift = Self::constant(cx, U256::from(shift));
        Self::from_kind(cx, SymExprKind::BinOp(SymExprBinOp::Shr, expr, shift))
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
            SymExprKind::BinOp(SymExprBinOp::Or, left, right) => {
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
            SymExprKind::BinOp(SymExprBinOp::Shl, byte, shift) => {
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
        let SymExprKind::BinOp(SymExprBinOp::Shr, source, shift) = expr.kind() else { return None };
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
        let SymExprKind::BinOp(SymExprBinOp::And, left, right) = self.kind() else { return None };
        if let Some(mask) = right.as_const() {
            return mask_low_bits(mask).map(|bits| (left.clone(), bits));
        }
        let mask = left.as_const()?;
        mask_low_bits(mask).map(|bits| (right.clone(), bits))
    }

    fn shifted_high_word_fragment(&self) -> Option<(Self, usize)> {
        let SymExprKind::BinOp(SymExprBinOp::Shl, value, shift) = self.kind() else { return None };
        let bits = shift.as_const().and_then(|shift| usize::try_from(shift).ok())?;
        if bits == 0 || bits >= 256 {
            return None;
        }

        let (source, source_shift, width) = value.shifted_low_fragment_source()?;
        (source_shift == bits && width == 256 - bits).then_some((source, bits))
    }

    fn shifted_low_fragment_source(&self) -> Option<(Self, usize, usize)> {
        let SymExprKind::BinOp(SymExprBinOp::And, left, right) = self.kind() else { return None };
        if let Some(mask) = right.as_const() {
            return Self::shifted_low_fragment_source_with_mask(left, mask);
        }
        let mask = left.as_const()?;
        Self::shifted_low_fragment_source_with_mask(right, mask)
    }

    fn shifted_low_fragment_source_with_mask(
        value: &Self,
        mask: U256,
    ) -> Option<(Self, usize, usize)> {
        let width = mask_low_bits(mask)?;
        match value.kind() {
            SymExprKind::BinOp(SymExprBinOp::Shr, source, shift) => {
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
        Self::binop(cx, SymExprBinOp::And, self, mask)
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
                Self::binop(cx, SymExprBinOp::Shl, byte, shift)
            };
            expr = Self::binop(cx, SymExprBinOp::Or, expr, byte);
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
        Ok(match self.kind() {
            SymExprKind::Const(value) => *value,
            SymExprKind::Var(var) => model.value(var.clone()).unwrap_or_default(),
            SymExprKind::GasLeft(_) => {
                return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
            }
            SymExprKind::Keccak { len, bytes, .. } => {
                let len = len.eval_model(model)?;
                let Ok(len) = usize::try_from(len) else {
                    return Err(SymbolicError::Solver(
                        "solver model uses an invalid keccak length".to_string(),
                    ));
                };
                if len > bytes.len() {
                    return Err(SymbolicError::Solver(
                        "solver model uses an invalid keccak length".to_string(),
                    ));
                }

                let mut input = Vec::with_capacity(len);
                for byte in bytes.iter().take(len) {
                    input.push((byte.eval_model(model)? & U256::from(0xff)).to::<u8>());
                }

                U256::from_be_bytes(keccak256(input).0)
            }
            SymExprKind::Hash { name, .. } => model.value(name.clone()).unwrap_or_default(),
            SymExprKind::Not(value) => !value.eval_model(model)?,
            SymExprKind::BinOp(op, left, right) => {
                op.eval(left.eval_model(model)?, right.eval_model(model)?)
            }
            SymExprKind::TernOp(op, left, right, modulus) => op.eval(
                left.eval_model(model)?,
                right.eval_model(model)?,
                modulus.eval_model(model)?,
            ),
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                if cond.eval_model(model)? {
                    then_expr.eval_model(model)?
                } else {
                    else_expr.eval_model(model)?
                }
            }
        })
    }

    pub(crate) fn eval_model_if_complete<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Option<U256>, SymbolicError> {
        let mut vars = SymbolicVars::default();
        self.collect_eval_vars(&mut vars);
        if vars.iter().cloned().all(|var| model.contains_name(var)) {
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
                    model.insert(var.clone(), value);
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

    fn bool_word_condition_from_parts(
        condition: &SymBoolExpr,
        then_expr: &Self,
        else_expr: &Self,
    ) -> Option<SymBoolExpr> {
        match (then_expr.as_const(), else_expr.as_const()) {
            (Some(then_value), Some(else_value))
                if then_value == U256::from(1) && else_value.is_zero() =>
            {
                Some(condition.clone())
            }
            (Some(then_value), Some(else_value))
                if then_value.is_zero() && else_value == U256::from(1) =>
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
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::BinOp(SymExprBinOp::UDiv, _, _)))
    }

    pub(crate) fn contains_ite(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Ite(_, _, _)))
    }

    pub(in crate::runtime) fn udiv_operands(&self) -> Option<(&Self, &Self)> {
        match self.kind() {
            SymExprKind::BinOp(SymExprBinOp::UDiv, numerator, denominator) => {
                Some((numerator, denominator))
            }
            _ => None,
        }
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit(&mut |expr| {
            match expr.kind() {
                SymExprKind::Var(var) | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(var.clone());
                }
                _ => {}
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
                SymExprBinOp::And => match (left.known_byte(index), right.known_byte(index)) {
                    (Some(left), Some(right)) => Some(left & right),
                    (Some(0), _) | (_, Some(0)) => Some(0),
                    _ => None,
                },
                SymExprBinOp::Or => Some(left.known_byte(index)? | right.known_byte(index)?),
                SymExprBinOp::Xor => Some(left.known_byte(index)? ^ right.known_byte(index)?),
                SymExprBinOp::Shl => {
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
                SymExprBinOp::Shr => {
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
                SymExprBinOp::Add
                | SymExprBinOp::Sub
                | SymExprBinOp::Mul
                | SymExprBinOp::UDiv
                | SymExprBinOp::URem
                | SymExprBinOp::SDiv
                | SymExprBinOp::SRem
                | SymExprBinOp::Sar => None,
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
        match self.kind() {
            SymExprKind::Const(value) => value.bit_len().max(1),
            SymExprKind::BinOp(SymExprBinOp::And, left, right) => {
                if let Some(mask) = right.as_const() {
                    left.unsigned_bits().min(mask.bit_len())
                } else if let Some(mask) = left.as_const() {
                    right.unsigned_bits().min(mask.bit_len())
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymExprBinOp::Add, left, right) => {
                left.unsigned_bits().max(right.unsigned_bits()).saturating_add(1).min(256)
            }
            SymExprKind::BinOp(SymExprBinOp::Mul, left, right) => {
                left.unsigned_bits().saturating_add(right.unsigned_bits()).min(256)
            }
            SymExprKind::BinOp(SymExprBinOp::Shl, left, right) => {
                if let Some(shift) = right.as_const().and_then(|shift| usize::try_from(shift).ok())
                {
                    left.unsigned_bits().saturating_add(shift).min(256)
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymExprBinOp::Shr, left, right) => {
                if let Some(shift) = right.as_const().and_then(|shift| usize::try_from(shift).ok())
                {
                    left.unsigned_bits().saturating_sub(shift).max(1)
                } else {
                    256
                }
            }
            SymExprKind::BinOp(SymExprBinOp::UDiv, left, _) => left.unsigned_bits(),
            SymExprKind::TernOp(_, _, _, modulus) => modulus.unsigned_bits(),
            SymExprKind::Ite(_, left, right) => left.unsigned_bits().max(right.unsigned_bits()),
            _ => 256,
        }
    }

    pub(crate) fn extracted_byte(&self, cx: &mut SymCx, index: usize) -> Self {
        debug_assert!(index < 32);
        let shift = Self::constant(cx, U256::from((31 - index) * 8));
        let shifted = Self::binop(cx, SymExprBinOp::Shr, self.clone(), shift);
        let mask = Self::constant(cx, U256::from(0xff));
        Self::binop(cx, SymExprBinOp::And, shifted, mask)
    }

    pub(crate) fn extracted_byte_source(&self, index: usize) -> Option<Self> {
        let expr = self.strip_low_byte_mask();
        if index == 31 {
            return Some(expr.clone());
        }
        let SymExprKind::BinOp(SymExprBinOp::Shr, source, shift) = expr.kind() else { return None };
        let shift = shift.as_const()?;
        (shift == U256::from((31 - index) * 8)).then(|| source.clone())
    }

    pub(crate) fn strip_low_byte_mask(&self) -> &Self {
        match self.kind() {
            SymExprKind::BinOp(SymExprBinOp::And, left, right)
                if right.as_const() == Some(U256::from(0xff)) =>
            {
                left.strip_low_byte_mask()
            }
            SymExprKind::BinOp(SymExprBinOp::And, left, right)
                if left.as_const() == Some(U256::from(0xff)) =>
            {
                right.strip_low_byte_mask()
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
                SymExprBinOp::And => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymExprBinOp::And,
                    |byte| byte == 0xff,
                    |byte| byte == 0,
                ),
                SymExprBinOp::Or => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymExprBinOp::Or,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymExprBinOp::Xor => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymExprBinOp::Xor,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymExprBinOp::Shl => {
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
                SymExprBinOp::Shr => {
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
                SymExprBinOp::Add
                | SymExprBinOp::Sub
                | SymExprBinOp::Mul
                | SymExprBinOp::UDiv
                | SymExprBinOp::URem
                | SymExprBinOp::SDiv
                | SymExprBinOp::SRem
                | SymExprBinOp::Sar => None,
            },
            SymExprKind::TernOp(_, _, _, _) => None,
        }
    }

    fn binary_byte_term(
        cx: &mut SymCx,
        left: &Self,
        right: &Self,
        index: usize,
        op: SymExprBinOp,
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
            SymExprKind::BinOp(SymExprBinOp::Or, left, right) => {
                if left.eval().is_some_and(|value| value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::BinOp(SymExprBinOp::And, left, right) => {
                if left.eval().is_some_and(|value| !value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| !value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::BinOp(SymExprBinOp::Shl | SymExprBinOp::Shr, value, shift)
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
                Self::binop(cx, SymExprBinOp::Add, expr, value)
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
    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    pub(in crate::runtime::expr) fn write_smt(&self, out: &mut String) {
        match self.kind() {
            SymExprKind::Const(value) => {
                let _ = write!(out, "(_ bv{value} 256)");
            }
            SymExprKind::Var(var) => out.push_str(var.as_str()),
            SymExprKind::GasLeft(id) => {
                let _ = write!(out, "gasleft_{id}");
            }
            SymExprKind::Keccak { name, .. } => out.push_str(name.as_str()),
            SymExprKind::Hash { name, .. } => out.push_str(name.as_str()),
            SymExprKind::Not(value) => {
                out.push_str("(bvnot ");
                value.write_smt(out);
                out.push(')');
            }
            SymExprKind::BinOp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymExprKind::TernOp(op, left, right, modulus) => {
                write_smt_wide_modular_arithmetic(out, op.smt(), left, right, modulus);
            }
            SymExprKind::Ite(cond, left, right) => {
                out.push_str("(ite ");
                cond.write_smt(out);
                out.push(' ');
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
        }
    }
}

fn write_smt_wide_modular_arithmetic(
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
    modulus.write_smt(out);
    out.push_str(" (_ bv0 256)) (_ bv0 256) ((_ extract 255 0) (bvurem (");
    out.push_str(op);
    out.push_str(" ((_ zero_extend 256) ");
    left.write_smt(out);
    out.push_str(") ((_ zero_extend 256) ");
    right.write_smt(out);
    out.push_str(")) ((_ zero_extend 256) ");
    modulus.write_smt(out);
    out.push_str("))))");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum SymExprTernOp {
    AddMod,
    MulMod,
}

impl SymExprTernOp {
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
pub(crate) enum SymExprBinOp {
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

impl SymExprBinOp {
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
