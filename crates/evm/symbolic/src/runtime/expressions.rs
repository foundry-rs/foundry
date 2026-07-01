use super::*;

pub(crate) fn keccak_word(bytes: Vec<SymExpr>) -> SymExpr {
    let len = bytes.len();
    keccak_word_with_len(bytes, SymExpr::constant(U256::from(len)))
}

pub(crate) fn keccak_word_with_len(bytes: Vec<SymExpr>, len: SymExpr) -> SymExpr {
    if let Some(len) = len.as_const()
        && let Ok(len) = usize::try_from(len)
        && len <= bytes.len()
        && let Ok(bytes) = concrete_expr_bytes(&bytes[..len], "symbolic keccak input")
    {
        return SymExpr::constant(U256::from_be_bytes(keccak256(bytes).0));
    }

    let exprs = bytes;
    let name = stable_symbol("keccak", format!("{len:?}:{exprs:?}").as_bytes());
    SymExpr::keccak_symbol(name, len, exprs)
}

pub(crate) fn symbolic_hash_word_with_len(
    algorithm: &'static str,
    bytes: Vec<SymExpr>,
    len: SymExpr,
) -> SymExpr {
    let exprs = bytes;
    let name = stable_symbol(algorithm, format!("{len:?}:{exprs:?}").as_bytes());
    let mut identity = Vec::with_capacity(exprs.len() + 1);
    identity.push(len);
    identity.extend(exprs);
    SymExpr::hash_symbol(name, algorithm, identity)
}

pub(crate) fn create2_address_word(
    state: &mut PathState,
    creator: Address,
    salt: SymExpr,
    initcode: &SymCode,
) -> Result<(SymExpr, Address), SymbolicError> {
    match (salt.as_const(), initcode.concrete_bytes("symbolic CREATE2 initcode")) {
        (Some(salt), Ok(initcode)) => {
            let address = creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode);
            Ok((SymExpr::constant(address_word(address)), address))
        }
        (None, Ok(initcode)) => {
            let initcode_hash = keccak256(&initcode);
            let word = symbolic_create2_address_word(
                state,
                format!("{creator:?}"),
                salt,
                format!("{initcode_hash:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (_, Err(SymbolicError::Unsupported("symbolic CREATE2 initcode"))) => {
            let initcode_bytes = initcode.read_byte_exprs(0, initcode.len());
            let word = symbolic_create2_address_word(
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
    state: &mut PathState,
    deployer: SymExpr,
    salt: SymExpr,
    init_code_hash: SymExpr,
) -> Result<SymExpr, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let salt_concrete = state.constrained_word(&salt);
    let init_code_hash_concrete = state.constrained_word(&init_code_hash);

    if let (Some(deployer), Some(salt), Some(init_code_hash)) =
        (deployer_concrete, salt_concrete, init_code_hash_concrete)
    {
        let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
        let address = deployer.create2(B256::from(salt.to_be_bytes::<32>()), init_code_hash);
        return Ok(SymExpr::constant(address_word(address)));
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

    Ok(symbolic_create2_address_word(state, deployer_identity, salt, init_code_hash_identity))
}

pub(crate) fn compute_create_address_word(
    state: &mut PathState,
    deployer: SymExpr,
    nonce: SymExpr,
) -> Result<SymExpr, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let nonce_concrete = state.constrained_word(&nonce);

    if let (Some(deployer), Some(nonce)) = (deployer_concrete, nonce_concrete) {
        let Ok(nonce) = u64::try_from(nonce) else {
            return Err(SymbolicError::Unsupported("symbolic vm.computeCreateAddress nonce"));
        };
        return Ok(SymExpr::constant(address_word(deployer.create(nonce))));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{deployer:?}"));
    Ok(symbolic_create_address_word(state, deployer_identity, nonce))
}

pub(crate) fn symbolic_create_address_word(
    state: &mut PathState,
    creator_identity: String,
    nonce: SymExpr,
) -> SymExpr {
    let name = stable_symbol("create_address", format!("{creator_identity}:{nonce:?}").as_bytes());
    let word = SymExpr::var_symbol(name);
    state.constraints.push(SymBoolExpr::cmp_word_const(
        SymBoolExprOp::Ult,
        &word,
        U256::from(1) << 160,
    ));
    word
}

pub(crate) fn symbolic_create2_address_word(
    state: &mut PathState,
    creator_identity: String,
    salt: SymExpr,
    initcode_identity: String,
) -> SymExpr {
    let name = stable_symbol(
        "create2_address",
        format!("{creator_identity}:{salt:?}:{initcode_identity}").as_bytes(),
    );
    let word = SymExpr::var_symbol(name);
    state.constraints.push(SymBoolExpr::cmp_word_const(
        SymBoolExprOp::Ult,
        &word,
        U256::from(1) << 160,
    ));
    word
}

impl SymExpr {
    pub(crate) fn select_storage_write(
        self,
        write_key: Self,
        write_value: Self,
        base: Self,
    ) -> Self {
        if write_value == base {
            return base;
        }
        let condition = self.storage_key_eq(&write_key);
        match condition.as_const() {
            Some(true) => write_value,
            Some(false) => base,
            None => Self::ite(condition, write_value, base),
        }
    }

    pub(crate) fn storage_key_eq(&self, write_key: &Self) -> SymBoolExpr {
        if let (Some(read_root), Some(write_root)) =
            (self.storage_mapping_root_slot(), write_key.storage_mapping_root_slot())
            && read_root != write_root
        {
            return SymBoolExpr::constant(false);
        }
        match (self.storage_layout_key(), write_key.storage_layout_key()) {
            (Some((read_base, read_offset)), Some((write_base, write_offset))) => {
                SymBoolExpr::and(vec![
                    SymBoolExpr::eq(read_base, write_base),
                    SymBoolExpr::eq(read_offset, write_offset),
                ])
            }
            (Some(_), None) if write_key.as_const().is_some() => SymBoolExpr::constant(false),
            (None, Some(_)) if self.as_const().is_some() => SymBoolExpr::constant(false),
            _ => SymBoolExpr::eq(self.clone(), write_key.clone()),
        }
    }

    fn storage_mapping_root_slot(&self) -> Option<U256> {
        let SymExprKind::Keccak { len, bytes, .. } = self.kind() else { return None };
        if len.as_const() != Some(U256::from(64)) || bytes.len() < 64 {
            return None;
        }

        let slot = Self::from_bytes(bytes[32..64].iter().cloned());
        match slot.kind() {
            SymExprKind::Const(slot) => Some(*slot),
            SymExprKind::Keccak { .. } => slot.storage_mapping_root_slot(),
            _ => None,
        }
    }

    fn storage_layout_key(&self) -> Option<(Self, Self)> {
        match self.kind() {
            SymExprKind::Keccak { .. } => Some((self.clone(), Self::zero())),
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                if let Some((base, offset)) = left.storage_layout_key()
                    && !right.contains_keccak()
                {
                    return Some((base, Self::op(SymExprOp::Add, offset, right.clone())));
                }
                if let Some((base, offset)) = right.storage_layout_key()
                    && !left.contains_keccak()
                {
                    return Some((base, Self::op(SymExprOp::Add, offset, left.clone())));
                }
                None
            }
            _ => None,
        }
    }
}

fn masked_expr_matches(candidate: &SymExprKind, target: &SymExpr) -> Option<U256> {
    match candidate {
        SymExprKind::Op(SymExprOp::And, left, right) if left == target => right.eval(),
        SymExprKind::Op(SymExprOp::And, left, right) if right == target => left.eval(),
        _ => None,
    }
}

fn context_forces_masked_expr(context: &[SymBoolExpr], target: &SymExpr, mask: U256) -> bool {
    context.iter().any(|condition| match condition.kind() {
        SymBoolExprKind::Eq(left, right) => {
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

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymExpr(Arc<SymExprKind>);

impl fmt::Debug for SymExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum SymExprKind {
    Const(U256),
    Var(Symbol),
    GasLeft(usize),
    Keccak { name: Symbol, len: SymExpr, bytes: Arc<[SymExpr]> },
    Hash { name: Symbol, algorithm: &'static str, bytes: Arc<[SymExpr]> },
    Not(SymExpr),
    Op(SymExprOp, SymExpr, SymExpr),
    AddMod { left: SymExpr, right: SymExpr, modulus: SymExpr },
    MulMod { left: SymExpr, right: SymExpr, modulus: SymExpr },
    Ite(SymBoolExpr, SymExpr, SymExpr),
}

static EXPR_ZERO: LazyLock<Arc<SymExprKind>> =
    LazyLock::new(|| Arc::new(SymExprKind::Const(U256::ZERO)));
static EXPR_ONE: LazyLock<Arc<SymExprKind>> =
    LazyLock::new(|| Arc::new(SymExprKind::Const(U256::from(1))));
static EXPR_MAX: LazyLock<Arc<SymExprKind>> =
    LazyLock::new(|| Arc::new(SymExprKind::Const(U256::MAX)));

impl SymExpr {
    fn from_kind(expr: SymExprKind) -> Self {
        match expr {
            SymExprKind::Const(value) => Self::constant(value),
            expr => Self(Arc::new(expr)),
        }
    }

    pub(crate) fn zero() -> Self {
        Self(EXPR_ZERO.clone())
    }

    pub(super) fn kind(&self) -> &SymExprKind {
        self.0.as_ref()
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

    pub(super) fn into_kind(self) -> SymExprKind {
        Arc::unwrap_or_clone(self.0)
    }

    pub(crate) fn constant(value: U256) -> Self {
        if value.is_zero() {
            Self(EXPR_ZERO.clone())
        } else if value == U256::from(1) {
            Self(EXPR_ONE.clone())
        } else if value == U256::MAX {
            Self(EXPR_MAX.clone())
        } else {
            Self(Arc::new(SymExprKind::Const(value)))
        }
    }

    pub(crate) fn low_byte(self) -> Self {
        if let Some(word) = self.as_const() {
            return Self::constant(U256::from(word.to::<u8>()));
        }
        Self::op(SymExprOp::And, self, Self::constant(U256::from(0xff)))
    }

    pub(crate) fn into_byte_exprs(self) -> Vec<Self> {
        SymBytes::word(self).materialize()
    }

    pub(crate) fn into_bytes(self) -> SymBytes {
        SymBytes::word(self)
    }

    pub(crate) fn from_bytes(bytes: impl IntoIterator<Item = Self>) -> Self {
        let bytes = bytes.into_iter().collect::<Vec<_>>();
        if let Ok(concrete) = concrete_expr_bytes(&bytes, "symbolic word bytes") {
            let mut word = [0u8; 32];
            for (idx, byte) in concrete.into_iter().take(32).enumerate() {
                word[idx] = byte;
            }
            return Self::constant(U256::from_be_bytes(word));
        }

        if let Some(expr) = word_from_extracted_bytes(&bytes) {
            return expr;
        }

        let mut expr = Self::zero();
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            let shift = (31 - idx) * 8;
            let byte = byte.low_byte();
            let byte = if shift == 0 {
                byte
            } else {
                Self::op(SymExprOp::Shl, byte, Self::constant(U256::from(shift)))
            };
            expr = Self::op(SymExprOp::Or, expr, byte);
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
        match self.kind() {
            SymExprKind::Const(value) => Some(*value),
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => None,
            SymExprKind::Not(value) => Some(!value.eval()?),
            SymExprKind::Op(op, left, right) => Some(op.eval(left.eval()?, right.eval()?)),
            SymExprKind::AddMod { left, right, modulus } => {
                Some(left.eval()?.add_mod(right.eval()?, modulus.eval()?))
            }
            SymExprKind::MulMod { left, right, modulus } => {
                Some(left.eval()?.mul_mod(right.eval()?, modulus.eval()?))
            }
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                if cond.eval()? {
                    then_expr.eval()
                } else {
                    else_expr.eval()
                }
            }
        }
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<U256, SymbolicError> {
        Ok(match self.kind() {
            SymExprKind::Const(value) => *value,
            SymExprKind::Var(var) => model.value(*var).unwrap_or_default(),
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
            SymExprKind::Hash { name, .. } => model.value(*name).unwrap_or_default(),
            SymExprKind::Not(value) => !value.eval_model(model)?,
            SymExprKind::Op(op, left, right) => {
                op.eval(left.eval_model(model)?, right.eval_model(model)?)
            }
            SymExprKind::AddMod { left, right, modulus } => left
                .eval_model(model)?
                .add_mod(right.eval_model(model)?, modulus.eval_model(model)?),
            SymExprKind::MulMod { left, right, modulus } => left
                .eval_model(model)?
                .mul_mod(right.eval_model(model)?, modulus.eval_model(model)?),
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                if cond.eval_model(model)? {
                    then_expr.eval_model(model)?
                } else {
                    else_expr.eval_model(model)?
                }
            }
        })
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
            _ => false,
        }
    }

    pub(crate) fn from_bool(value: SymBoolExpr) -> Self {
        Self::bool_word(value)
    }

    pub(crate) fn bool_word(value: SymBoolExpr) -> Self {
        Self::ite(value, Self::constant(U256::from(1)), Self::zero())
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
                Some(condition.clone().not())
            }
            _ => None,
        }
    }

    pub(crate) fn truth(&self) -> Option<bool> {
        self.as_const().map(|value| !value.is_zero())
    }

    pub(crate) fn into_zero_bool(self) -> SymBoolExpr {
        match self.kind() {
            SymExprKind::Const(value) => SymBoolExpr::constant(value.is_zero()),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                match Self::bool_word_condition_from_parts(condition, then_expr, else_expr) {
                    Some(condition) => condition.not(),
                    None => SymBoolExpr::eq(self, Self::zero()),
                }
            }
            _ => SymBoolExpr::eq(self, Self::zero()),
        }
    }

    pub(crate) fn nonzero_bool(self) -> SymBoolExpr {
        self.into_zero_bool().not()
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
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Op(SymExprOp::UDiv, _, _)))
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit(&mut |expr| {
            match expr.kind() {
                SymExprKind::Var(var) | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(*var);
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
            SymExprKind::Op(op, left, right) => match op {
                SymExprOp::And => match (left.known_byte(index), right.known_byte(index)) {
                    (Some(left), Some(right)) => Some(left & right),
                    (Some(0), _) | (_, Some(0)) => Some(0),
                    _ => None,
                },
                SymExprOp::Or => Some(left.known_byte(index)? | right.known_byte(index)?),
                SymExprOp::Xor => Some(left.known_byte(index)? ^ right.known_byte(index)?),
                SymExprOp::Shl => {
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
                SymExprOp::Shr => {
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
                SymExprOp::Add
                | SymExprOp::Sub
                | SymExprOp::Mul
                | SymExprOp::UDiv
                | SymExprOp::URem
                | SymExprOp::SDiv
                | SymExprOp::SRem
                | SymExprOp::Sar => None,
            },
            SymExprKind::AddMod { .. } | SymExprKind::MulMod { .. } => None,
        }
    }

    pub(crate) fn known_word(&self) -> Option<U256> {
        let mut word = [0u8; 32];
        for (idx, byte) in word.iter_mut().enumerate() {
            *byte = self.known_byte(idx)?;
        }
        Some(U256::from_be_bytes(word))
    }

    pub(crate) fn extracted_byte(&self, index: usize) -> Self {
        debug_assert!(index < 32);
        Self::op(
            SymExprOp::And,
            Self::op(SymExprOp::Shr, self.clone(), Self::constant(U256::from((31 - index) * 8))),
            Self::constant(U256::from(0xff)),
        )
    }

    pub(crate) fn extracted_byte_source(&self, index: usize) -> Option<Self> {
        let expr = self.strip_low_byte_mask();
        if index == 31 {
            return Some(expr.clone());
        }
        let SymExprKind::Op(SymExprOp::Shr, source, shift) = expr.kind() else { return None };
        let shift = shift.as_const()?;
        (shift == U256::from((31 - index) * 8)).then(|| source.clone())
    }

    pub(crate) fn strip_low_byte_mask(&self) -> &Self {
        match self.kind() {
            SymExprKind::Op(SymExprOp::And, left, right)
                if right.as_const() == Some(U256::from(0xff)) =>
            {
                left.strip_low_byte_mask()
            }
            SymExprKind::Op(SymExprOp::And, left, right)
                if left.as_const() == Some(U256::from(0xff)) =>
            {
                right.strip_low_byte_mask()
            }
            _ => self,
        }
    }

    pub(crate) fn byte_term(&self, index: usize) -> Option<Self> {
        debug_assert!(index < 32);

        match self.kind() {
            SymExprKind::Const(value) => {
                Some(Self::constant(U256::from(value.to_be_bytes::<32>()[index])))
            }
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => Some(self.extracted_byte(index)),
            SymExprKind::Not(value) => Some(Self::not(value.byte_term(index)?)),
            SymExprKind::Ite(cond, then_expr, else_expr) => Some(Self::ite(
                cond.clone(),
                then_expr.byte_term(index)?,
                else_expr.byte_term(index)?,
            )),
            SymExprKind::Op(op, left, right) => match op {
                SymExprOp::And => Self::binary_byte_term(
                    left,
                    right,
                    index,
                    SymExprOp::And,
                    |byte| byte == 0xff,
                    |byte| byte == 0,
                ),
                SymExprOp::Or => Self::binary_byte_term(
                    left,
                    right,
                    index,
                    SymExprOp::Or,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymExprOp::Xor => Self::binary_byte_term(
                    left,
                    right,
                    index,
                    SymExprOp::Xor,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymExprOp::Shl => {
                    let shift = right.eval()?;
                    if shift >= U256::from(256) {
                        return Some(Self::zero());
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let source_index = index + shift / 8;
                    if source_index >= 32 {
                        Some(Self::zero())
                    } else {
                        left.byte_term(source_index)
                    }
                }
                SymExprOp::Shr => {
                    let shift = right.eval()?;
                    if shift >= U256::from(256) {
                        return Some(Self::zero());
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let byte_shift = shift / 8;
                    if index < byte_shift {
                        Some(Self::zero())
                    } else {
                        left.byte_term(index - byte_shift)
                    }
                }
                SymExprOp::Add
                | SymExprOp::Sub
                | SymExprOp::Mul
                | SymExprOp::UDiv
                | SymExprOp::URem
                | SymExprOp::SDiv
                | SymExprOp::SRem
                | SymExprOp::Sar => None,
            },
            SymExprKind::AddMod { .. } | SymExprKind::MulMod { .. } => None,
        }
    }

    fn binary_byte_term(
        left: &Self,
        right: &Self,
        index: usize,
        op: SymExprOp,
        identity: impl Fn(u8) -> bool,
        absorbing: impl Fn(u8) -> bool,
    ) -> Option<Self> {
        let left = left.byte_term(index)?;
        let right = right.byte_term(index)?;
        match (left.byte_const(), right.byte_const()) {
            (Some(left), _) if absorbing(left) => Some(Self::constant(U256::from(left))),
            (_, Some(right)) if absorbing(right) => Some(Self::constant(U256::from(right))),
            (Some(left), _) if identity(left) => Some(right),
            (_, Some(right)) if identity(right) => Some(left),
            _ => Some(Self::op(op, left, right)),
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
            SymExprKind::Op(SymExprOp::Or, left, right) => {
                if left.eval().is_some_and(|value| value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::Op(SymExprOp::And, left, right) => {
                if left.eval().is_some_and(|value| !value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval().is_some_and(|value| !value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprKind::Op(SymExprOp::Shl | SymExprOp::Shr, value, shift)
                if shift.eval().is_some_and(|shift| shift.is_zero()) =>
            {
                value.nonzero_forces_const(target, context)
            }
            SymExprKind::AddMod { .. } | SymExprKind::MulMod { .. } => None,
            SymExprKind::Op(_, _, _) => None,
        }
    }

    pub(crate) fn is_raw_gasleft(&self) -> bool {
        matches!(self.kind(), SymExprKind::GasLeft(_))
    }

    pub(crate) fn var(name: &str) -> Self {
        Self::var_symbol(Symbol::intern(name))
    }

    pub(crate) fn var_symbol(name: Symbol) -> Self {
        Self::from_kind(SymExprKind::Var(name))
    }

    pub(crate) fn gas_left(id: usize) -> Self {
        Self::from_kind(SymExprKind::GasLeft(id))
    }

    pub(crate) fn keccak_symbol(name: Symbol, len: Self, bytes: Vec<Self>) -> Self {
        Self::from_kind(SymExprKind::Keccak { name, len, bytes: bytes.into() })
    }

    pub(crate) fn hash_symbol(name: Symbol, algorithm: &'static str, bytes: Vec<Self>) -> Self {
        Self::from_kind(SymExprKind::Hash { name, algorithm, bytes: bytes.into() })
    }

    pub(crate) fn ite(cond: SymBoolExpr, then_expr: Self, else_expr: Self) -> Self {
        match cond.as_const() {
            Some(true) => then_expr,
            Some(false) => else_expr,
            None if then_expr == else_expr => then_expr,
            None => Self::from_kind(SymExprKind::Ite(cond, then_expr, else_expr)),
        }
    }

    pub(crate) fn add_const(expr: Self, value: U256) -> Self {
        if value.is_zero() {
            return expr;
        }
        match expr.kind() {
            SymExprKind::Const(expr) => Self::constant(expr.wrapping_add(value)),
            _ => Self::from_kind(SymExprKind::Op(SymExprOp::Add, expr, Self::constant(value))),
        }
    }

    pub(crate) fn not(value: Self) -> Self {
        match value.kind() {
            SymExprKind::Const(value) => Self::constant(!*value),
            SymExprKind::Not(value) => value.clone(),
            _ => Self::from_kind(SymExprKind::Not(value)),
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
            SymExprKind::Op(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
            SymExprKind::AddMod { left, right, modulus }
            | SymExprKind::MulMod { left, right, modulus } => {
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

    pub(crate) fn fold(self, folder: &mut impl FnMut(Self) -> Self) -> Self {
        if matches!(
            self.kind(),
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_)
        ) {
            return folder(self);
        }

        let expr = match self.into_kind() {
            SymExprKind::Keccak { name, len, bytes } => Self::keccak_symbol(
                name,
                len.fold(folder),
                bytes.iter().cloned().map(|byte| byte.fold(folder)).collect(),
            ),
            SymExprKind::Hash { name, algorithm, bytes } => Self::hash_symbol(
                name,
                algorithm,
                bytes.iter().cloned().map(|byte| byte.fold(folder)).collect(),
            ),
            SymExprKind::Not(value) => Self::not(value.fold(folder)),
            SymExprKind::Op(op, left, right) => Self::op(op, left.fold(folder), right.fold(folder)),
            SymExprKind::AddMod { left, right, modulus } => {
                Self::addmod(left.fold(folder), right.fold(folder), modulus.fold(folder))
            }
            SymExprKind::MulMod { left, right, modulus } => {
                Self::mulmod(left.fold(folder), right.fold(folder), modulus.fold(folder))
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => Self::ite(
                condition.fold_exprs(folder),
                then_expr.fold(folder),
                else_expr.fold(folder),
            ),
            SymExprKind::Const(_) | SymExprKind::Var(_) | SymExprKind::GasLeft(_) => {
                unreachable!("leaf expression returned before folding children")
            }
        };
        folder(expr)
    }

    pub(crate) fn op(op: SymExprOp, left: Self, right: Self) -> Self {
        if let Some(expr) = Self::peephole_op(op, &left, &right) {
            return expr;
        }
        Self::from_kind(SymExprKind::Op(op, left, right))
    }

    fn peephole_op(op: SymExprOp, left: &Self, right: &Self) -> Option<Self> {
        match op {
            SymExprOp::Add => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (SymExprKind::Const(value), _) if value.is_zero() => Some(right.clone()),
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(left.clone()),
                _ => None,
            },
            SymExprOp::Sub => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(left.clone()),
                _ if left == right => Some(Self::zero()),
                _ => None,
            },
            SymExprOp::Mul => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    Some(Self::zero())
                }
                (SymExprKind::Const(value), _) if *value == U256::from(1) => Some(right.clone()),
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => Some(left.clone()),
                _ => None,
            },
            SymExprOp::UDiv | SymExprOp::SDiv => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(Self::zero()),
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => Some(left.clone()),
                _ => None,
            },
            SymExprOp::URem | SymExprOp::SRem => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(Self::zero()),
                (_, SymExprKind::Const(value)) if *value == U256::from(1) => Some(Self::zero()),
                _ => None,
            },
            SymExprOp::And => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (SymExprKind::Const(value), _) | (_, SymExprKind::Const(value))
                    if value.is_zero() =>
                {
                    Some(Self::zero())
                }
                (SymExprKind::Const(value), _) if *value == U256::MAX => Some(right.clone()),
                (_, SymExprKind::Const(value)) if *value == U256::MAX => Some(left.clone()),
                _ if left == right => Some(left.clone()),
                (SymExprKind::Const(mask), _) => Some(Self::and_const(right.clone(), *mask)),
                (_, SymExprKind::Const(mask)) => Some(Self::and_const(left.clone(), *mask)),
                _ => None,
            },
            SymExprOp::Or | SymExprOp::Xor => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (SymExprKind::Const(value), _) if value.is_zero() => Some(right.clone()),
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(left.clone()),
                _ => None,
            },
            SymExprOp::Shl | SymExprOp::Shr => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(left.clone()),
                (SymExprKind::Const(value), _) if value.is_zero() => Some(Self::zero()),
                _ => None,
            },
            SymExprOp::Sar => match (left.kind(), right.kind()) {
                (SymExprKind::Const(left_value), SymExprKind::Const(right_value)) => {
                    Some(Self::constant(op.eval(*left_value, *right_value)))
                }
                (_, SymExprKind::Const(value)) if value.is_zero() => Some(left.clone()),
                _ => None,
            },
        }
    }

    /// Builds an exact EVM `ADDMOD` expression.
    pub(crate) fn addmod(left: Self, right: Self, modulus: Self) -> Self {
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus))
                if modulus.is_zero() || *modulus == U256::from(1) =>
            {
                Self::zero()
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                Self::constant(left.add_mod(*right, *modulus))
            }
            _ => Self::from_kind(SymExprKind::AddMod { left, right, modulus }),
        }
    }

    /// Builds an exact EVM `MULMOD` expression.
    pub(crate) fn mulmod(left: Self, right: Self, modulus: Self) -> Self {
        match (left.kind(), right.kind(), modulus.kind()) {
            (_, _, SymExprKind::Const(modulus))
                if modulus.is_zero() || *modulus == U256::from(1) =>
            {
                Self::zero()
            }
            (SymExprKind::Const(left), SymExprKind::Const(right), SymExprKind::Const(modulus)) => {
                Self::constant(left.mul_mod(*right, *modulus))
            }
            _ => Self::from_kind(SymExprKind::MulMod { left, right, modulus }),
        }
    }

    fn and_const(expr: Self, mask: U256) -> Self {
        if mask.is_zero() {
            return Self::zero();
        }
        if mask == U256::MAX {
            return expr;
        }

        match expr.kind() {
            SymExprKind::Op(SymExprOp::And, left, right) => match (left.kind(), right.kind()) {
                (SymExprKind::Const(value), _) if *value == mask => {
                    Self::and_const(right.clone(), mask)
                }
                (_, SymExprKind::Const(value)) if *value == mask => {
                    Self::and_const(left.clone(), mask)
                }
                _ if left == right => Self::and_const(left.clone(), mask),
                _ => Self::from_kind(SymExprKind::Op(SymExprOp::And, expr, Self::constant(mask))),
            },
            _ => Self::from_kind(SymExprKind::Op(SymExprOp::And, expr, Self::constant(mask))),
        }
    }

    #[cfg(test)]
    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    fn write_smt(&self, out: &mut String) {
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
            SymExprKind::Op(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymExprKind::AddMod { left, right, modulus } => {
                write_smt_wide_modular_arithmetic(out, "bvadd", left, right, modulus);
            }
            SymExprKind::MulMod { left, right, modulus } => {
                write_smt_wide_modular_arithmetic(out, "bvmul", left, right, modulus);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SymExprOp {
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

impl SymExprOp {
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

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymBoolExpr(Arc<SymBoolExprKind>);

impl fmt::Debug for SymBoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum SymBoolExprKind {
    Const(bool),
    Not(SymBoolExpr),
    And(Arc<[SymBoolExpr]>),
    Eq(SymExpr, SymExpr),
    Cmp(SymBoolExprOp, SymExpr, SymExpr),
}

static BOOL_TRUE: LazyLock<Arc<SymBoolExprKind>> =
    LazyLock::new(|| Arc::new(SymBoolExprKind::Const(true)));
static BOOL_FALSE: LazyLock<Arc<SymBoolExprKind>> =
    LazyLock::new(|| Arc::new(SymBoolExprKind::Const(false)));

impl SymBoolExpr {
    fn from_kind(expr: SymBoolExprKind) -> Self {
        match expr {
            SymBoolExprKind::Const(value) => Self::constant(value),
            expr => Self(Arc::new(expr)),
        }
    }

    pub(crate) fn constant(value: bool) -> Self {
        Self(if value { &*BOOL_TRUE } else { &*BOOL_FALSE }.clone())
    }

    pub(super) fn kind(&self) -> &SymBoolExprKind {
        self.0.as_ref()
    }

    pub(super) fn into_kind(self) -> SymBoolExprKind {
        Arc::unwrap_or_clone(self.0)
    }

    pub(crate) fn as_const(&self) -> Option<bool> {
        match self.kind() {
            SymBoolExprKind::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn eval(&self) -> Option<bool> {
        match self.kind() {
            SymBoolExprKind::Const(value) => Some(*value),
            SymBoolExprKind::Not(value) => Some(!value.eval()?),
            SymBoolExprKind::And(values) => {
                let mut all_true = true;
                for value in values.iter() {
                    all_true &= value.eval()?;
                }
                Some(all_true)
            }
            SymBoolExprKind::Eq(left, right) => Some(left.eval()? == right.eval()?),
            SymBoolExprKind::Cmp(op, left, right) => Some(op.eval(left.eval()?, right.eval()?)),
        }
    }

    pub(crate) fn contains_keccak(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::Keccak { .. }))
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        self.visit_bool(|expr| matches!(expr.kind(), SymExprKind::GasLeft(_)))
    }

    pub(crate) fn contains_udiv(&self) -> bool {
        self.visit_bool(|expr| expr.contains_udiv())
    }

    pub(crate) fn forces_expr_const_with_context(
        &self,
        expr: &SymExpr,
        context: &[Self],
    ) -> Option<U256> {
        match self.kind() {
            SymBoolExprKind::Eq(left, right) => match (left.kind(), right.kind()) {
                (_, SymExprKind::Const(value)) => left.equality_forces_const(*value, expr, context),
                (SymExprKind::Const(value), _) => {
                    right.equality_forces_const(*value, expr, context)
                }
                _ => None,
            },
            SymBoolExprKind::Not(value) => match value.kind() {
                SymBoolExprKind::Eq(left, right) => match (left.kind(), right.kind()) {
                    (_, SymExprKind::Const(value)) if value.is_zero() => {
                        left.nonzero_forces_const(expr, context)
                    }
                    (SymExprKind::Const(value), _) if value.is_zero() => {
                        right.nonzero_forces_const(expr, context)
                    }
                    _ => None,
                },
                SymBoolExprKind::Not(value) => value.forces_expr_const_with_context(expr, context),
                _ => None,
            },
            SymBoolExprKind::And(values) => {
                values.iter().find_map(|value| value.forces_expr_const_with_context(expr, context))
            }
            _ => None,
        }
    }

    pub(crate) fn upper_bound_usize(&self, expr: &SymExpr) -> Option<usize> {
        match self.kind() {
            SymBoolExprKind::Const(_) | SymBoolExprKind::Not(_) => None,
            SymBoolExprKind::And(values) => {
                let mut bound: Option<usize> = None;
                for value in values.iter() {
                    if let Some(candidate) = value.upper_bound_usize(expr) {
                        bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                    }
                }
                bound
            }
            SymBoolExprKind::Eq(left, right) => match (left == expr, right == expr) {
                (true, _) => right.eval().and_then(|value| usize::try_from(value).ok()),
                (_, true) => left.eval().and_then(|value| usize::try_from(value).ok()),
                _ => None,
            },
            SymBoolExprKind::Cmp(op, left, right) => {
                if left == expr {
                    match *op {
                        SymBoolExprOp::Ult => right
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Ule => {
                            right.eval().and_then(|value| usize::try_from(value).ok())
                        }
                        _ => None,
                    }
                } else if right == expr {
                    match *op {
                        SymBoolExprOp::Ugt => left
                            .eval()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Uge => {
                            left.eval().and_then(|value| usize::try_from(value).ok())
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<bool, SymbolicError> {
        Ok(match self.kind() {
            SymBoolExprKind::Const(value) => *value,
            SymBoolExprKind::Not(value) => !value.eval_model(model)?,
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    if !value.eval_model(model)? {
                        return Ok(false);
                    }
                }
                true
            }
            SymBoolExprKind::Eq(left, right) => {
                left.eval_model(model)? == right.eval_model(model)?
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                op.eval(left.eval_model(model)?, right.eval_model(model)?)
            }
        })
    }

    pub(crate) fn eval_model_if_complete<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Option<bool>, SymbolicError> {
        let mut vars = SymbolicVars::default();
        self.collect_eval_vars(&mut vars);
        if vars.iter().copied().all(|var| model.contains_name(var)) {
            self.eval_model(model).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Visits all word expressions contained in this boolean expression.
    pub(crate) fn visit_exprs<B>(
        &self,
        visitor: &mut impl FnMut(&SymExpr) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.kind() {
            SymBoolExprKind::Const(_) => {}
            SymBoolExprKind::Not(value) => value.visit_exprs(visitor)?,
            SymBoolExprKind::And(values) => {
                for value in values.iter() {
                    value.visit_exprs(visitor)?;
                }
            }
            SymBoolExprKind::Eq(left, right) | SymBoolExprKind::Cmp(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn visit_bool(&self, mut visitor: impl FnMut(&SymExpr) -> bool) -> bool {
        self.visit_exprs(&mut |expr| {
            if visitor(expr) { ControlFlow::Break(()) } else { ControlFlow::Continue(()) }
        })
        .is_break()
    }

    pub(crate) fn fold(self, folder: &mut impl FnMut(Self) -> Self) -> Self {
        if matches!(self.kind(), SymBoolExprKind::Const(_)) {
            return folder(self);
        }

        let expr = match self.into_kind() {
            SymBoolExprKind::Not(value) => value.fold(folder).not(),
            SymBoolExprKind::And(values) => {
                Self::and(values.iter().cloned().map(|value| value.fold(folder)).collect())
            }
            SymBoolExprKind::Eq(left, right) => Self::eq(left, right),
            SymBoolExprKind::Cmp(op, left, right) => Self::cmp(op, left, right),
            SymBoolExprKind::Const(_) => unreachable!("leaf boolean returned before folding"),
        };
        folder(expr)
    }

    pub(crate) fn fold_exprs(self, folder: &mut impl FnMut(SymExpr) -> SymExpr) -> Self {
        if matches!(self.kind(), SymBoolExprKind::Const(_)) {
            return self;
        }

        match self.into_kind() {
            SymBoolExprKind::Not(value) => value.fold_exprs(folder).not(),
            SymBoolExprKind::And(values) => {
                Self::and(values.iter().cloned().map(|value| value.fold_exprs(folder)).collect())
            }
            SymBoolExprKind::Eq(left, right) => Self::eq(left.fold(folder), right.fold(folder)),
            SymBoolExprKind::Cmp(op, left, right) => {
                Self::cmp(op, left.fold(folder), right.fold(folder))
            }
            SymBoolExprKind::Const(_) => unreachable!("leaf boolean returned before folding exprs"),
        }
    }

    pub(crate) fn eq(left: SymExpr, right: SymExpr) -> Self {
        match (left.kind(), right.kind()) {
            _ if left == right => Self::constant(true),
            (SymExprKind::Const(left), SymExprKind::Const(right)) => Self::constant(left == right),
            (_, SymExprKind::Const(right_value)) => {
                if let Some(condition) = Self::bool_word_eq_const(&left, *right_value) {
                    return condition;
                }
                if let Some(left_value) = left.known_word() {
                    return Self::constant(left_value == *right_value);
                }
                Self::from_kind(SymBoolExprKind::Eq(left, right))
            }
            (SymExprKind::Const(left_value), _) => {
                if let Some(condition) = Self::bool_word_eq_const(&right, *left_value) {
                    return condition;
                }
                if let Some(right_value) = right.known_word() {
                    return Self::constant(*left_value == right_value);
                }
                Self::from_kind(SymBoolExprKind::Eq(left, right))
            }
            (
                SymExprKind::Keccak { len: left_len, bytes: left_bytes, .. },
                SymExprKind::Keccak { len: right_len, bytes: right_bytes, .. },
            ) if left_bytes.len() == right_bytes.len() => {
                let mut conditions = vec![Self::eq(left_len.clone(), right_len.clone())];
                conditions.extend(
                    left_bytes
                        .iter()
                        .cloned()
                        .zip(right_bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right)),
                );
                Self::and(conditions)
            }
            (
                SymExprKind::Hash { algorithm: left_algorithm, bytes: left_bytes, .. },
                SymExprKind::Hash { algorithm: right_algorithm, bytes: right_bytes, .. },
            ) if left_algorithm == right_algorithm && left_bytes.len() == right_bytes.len() => {
                Self::and(
                    left_bytes
                        .iter()
                        .cloned()
                        .zip(right_bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right))
                        .collect(),
                )
            }
            _ => Self::from_kind(SymBoolExprKind::Eq(left, right)),
        }
    }

    fn bool_word_eq_const(word: &SymExpr, value: U256) -> Option<Self> {
        let condition = word.bool_word_condition()?;
        Some(if value.is_zero() {
            condition.not()
        } else if value == U256::from(1) {
            condition
        } else {
            Self::constant(false)
        })
    }

    pub(crate) fn eq_word_const(word: &SymExpr, value: U256) -> Self {
        if let Some(word) = word.as_const() {
            Self::constant(word == value)
        } else {
            Self::eq(word.clone(), SymExpr::constant(value))
        }
    }

    pub(crate) fn eq_word_expr(word: &SymExpr, expr: SymExpr) -> Self {
        Self::eq(word.clone(), expr)
    }

    pub(crate) fn eq_words(left: &SymExpr, right: &SymExpr) -> Self {
        Self::eq(left.clone(), right.clone())
    }

    pub(crate) fn and(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                SymBoolExprKind::Const(true) => {}
                SymBoolExprKind::Const(false) => return Self::constant(false),
                SymBoolExprKind::And(values) => out.extend(values.iter().cloned()),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            Self::constant(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::from_kind(SymBoolExprKind::And(out.into()))
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_and(values: Vec<Self>) -> Self {
        Self::from_kind(SymBoolExprKind::And(values.into()))
    }

    pub(crate) fn or(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.kind() {
                SymBoolExprKind::Const(false) => {}
                SymBoolExprKind::Const(true) => return Self::constant(true),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            Self::constant(false)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::and(out.into_iter().map(Self::not).collect()).not()
        }
    }

    pub(crate) fn cmp(op: SymBoolExprOp, left: SymExpr, right: SymExpr) -> Self {
        match (op, left.kind(), right.kind()) {
            (op, _, _) if left == right => {
                Self::constant(matches!(op, SymBoolExprOp::Ule | SymBoolExprOp::Uge))
            }
            (op, SymExprKind::Const(left), SymExprKind::Const(right)) => {
                Self::constant(op.eval(*left, *right))
            }
            (SymBoolExprOp::Ugt, SymExprKind::Const(value), _) if value.is_zero() => {
                Self::constant(false)
            }
            (SymBoolExprOp::Ule, SymExprKind::Const(value), _) if value.is_zero() => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ult, _, SymExprKind::Const(value)) if value.is_zero() => {
                Self::constant(false)
            }
            (SymBoolExprOp::Uge, _, SymExprKind::Const(value)) if value.is_zero() => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ult, SymExprKind::Const(value), _) if *value == U256::MAX => {
                Self::constant(false)
            }
            (SymBoolExprOp::Uge, SymExprKind::Const(value), _) if *value == U256::MAX => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ugt, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                Self::constant(false)
            }
            (SymBoolExprOp::Ule, _, SymExprKind::Const(value)) if *value == U256::MAX => {
                Self::constant(true)
            }
            _ => Self::from_kind(SymBoolExprKind::Cmp(op, left, right)),
        }
    }

    pub(crate) fn cmp_word_const(op: SymBoolExprOp, word: &SymExpr, value: U256) -> Self {
        if let Some(word) = word.as_const() {
            Self::constant(op.eval(word, value))
        } else {
            Self::cmp(op, word.clone(), SymExpr::constant(value))
        }
    }

    pub(crate) fn cmp_word_expr(op: SymBoolExprOp, word: &SymExpr, expr: SymExpr) -> Self {
        Self::cmp(op, word.clone(), expr)
    }

    pub(crate) fn not(self) -> Self {
        match self.kind() {
            SymBoolExprKind::Const(value) => Self::constant(!*value),
            SymBoolExprKind::Not(value) => value.clone(),
            _ => Self::from_kind(SymBoolExprKind::Not(self)),
        }
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit_exprs(&mut |expr| {
            match expr.kind() {
                SymExprKind::Var(var)
                | SymExprKind::Keccak { name: var, .. }
                | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(*var);
                }
                _ => {}
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn collect_eval_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit_exprs(&mut |expr| {
            match expr.kind() {
                SymExprKind::Var(var) | SymExprKind::Hash { name: var, .. } => {
                    vars.insert(*var);
                }
                _ => {}
            }
            ControlFlow::<()>::Continue(())
        });
    }

    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    fn write_smt(&self, out: &mut String) {
        match self.kind() {
            SymBoolExprKind::Const(value) => out.push_str(if *value { "true" } else { "false" }),
            SymBoolExprKind::Not(value) => {
                out.push_str("(not ");
                value.write_smt(out);
                out.push(')');
            }
            SymBoolExprKind::And(values) => {
                out.push_str("(and");
                for value in values.iter() {
                    out.push(' ');
                    value.write_smt(out);
                }
                out.push(')');
            }
            SymBoolExprKind::Eq(left, right) => {
                out.push_str("(= ");
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymBoolExprKind::Cmp(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SymBoolExprOp {
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl SymBoolExprOp {
    pub(crate) const fn smt(self) -> &'static str {
        match self {
            Self::Ult => "bvult",
            Self::Ugt => "bvugt",
            Self::Ule => "bvule",
            Self::Uge => "bvuge",
            Self::Slt => "bvslt",
            Self::Sgt => "bvsgt",
        }
    }

    pub(crate) fn eval(self, left: U256, right: U256) -> bool {
        match self {
            Self::Ult => left < right,
            Self::Ugt => left > right,
            Self::Ule => left <= right,
            Self::Uge => left >= right,
            Self::Slt => slt(left, right),
            Self::Sgt => slt(right, left),
        }
    }
}
