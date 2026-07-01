use super::{hashcons::HashConsed, *};
use std::hash::{Hash, Hasher};

pub(crate) fn keccak_word(cx: &mut SymCx, bytes: Vec<SymExpr>) -> SymExpr {
    let len = bytes.len();
    let len = cx.constant(U256::from(len));
    keccak_word_with_len(cx, bytes, len)
}

pub(crate) fn keccak_word_with_len(cx: &mut SymCx, bytes: Vec<SymExpr>, len: SymExpr) -> SymExpr {
    if let Some(len) = len.as_const()
        && let Ok(len) = usize::try_from(len)
        && len <= bytes.len()
        && let Ok(bytes) = concrete_expr_bytes(&bytes[..len], "symbolic keccak input")
    {
        return cx.constant(U256::from_be_bytes(keccak256(bytes).0));
    }

    let exprs = bytes;
    let name = stable_symbol("keccak", format!("{len:?}:{exprs:?}").as_bytes());
    cx.keccak_symbol(name, len, exprs)
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
    cx.hash_symbol(name, algorithm, identity)
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
            Ok((cx.constant(address_word(address)), address))
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
        return Ok(cx.constant(address_word(address)));
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
        return Ok(cx.constant(address_word(deployer.create(nonce))));
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
    let word = cx.var_symbol(name);
    state.constraints.push(cx.cmp_word_const(SymBoolExprOp::Ult, &word, U256::from(1) << 160));
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
    let word = cx.var_symbol(name);
    state.constraints.push(cx.cmp_word_const(SymBoolExprOp::Ult, &word, U256::from(1) << 160));
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
            None => cx.ite(condition, write_value, base),
        }
    }

    pub(crate) fn storage_key_eq(&self, cx: &mut SymCx, write_key: &Self) -> SymBoolExpr {
        if let (Some(read_root), Some(write_root)) =
            (self.storage_mapping_root_slot(cx), write_key.storage_mapping_root_slot(cx))
            && read_root != write_root
        {
            return cx.bool_constant(false);
        }
        match (self.storage_layout_key(cx), write_key.storage_layout_key(cx)) {
            (Some((read_base, read_offset)), Some((write_base, write_offset))) => {
                let read_base = cx.eq(read_base, write_base);
                let read_offset = cx.eq(read_offset, write_offset);
                cx.and(vec![read_base, read_offset])
            }
            (Some(_), None) if write_key.as_const().is_some() => cx.bool_constant(false),
            (None, Some(_)) if self.as_const().is_some() => cx.bool_constant(false),
            _ => cx.eq(self.clone(), write_key.clone()),
        }
    }

    fn storage_mapping_root_slot(&self, cx: &mut SymCx) -> Option<U256> {
        let SymExprKind::Keccak { len, bytes, .. } = self.kind() else { return None };
        if len.as_const() != Some(U256::from(64)) || bytes.len() < 64 {
            return None;
        }

        let slot = cx.from_bytes(bytes[32..64].iter().cloned());
        match slot.kind() {
            SymExprKind::Const(slot) => Some(*slot),
            SymExprKind::Keccak { .. } => slot.storage_mapping_root_slot(cx),
            _ => None,
        }
    }

    fn storage_layout_key(&self, cx: &mut SymCx) -> Option<(Self, Self)> {
        match self.kind() {
            SymExprKind::Keccak { .. } => Some((self.clone(), cx.zero())),
            SymExprKind::Op(SymExprOp::Add, left, right) => {
                if let Some((base, offset)) = left.storage_layout_key(cx)
                    && !right.contains_keccak()
                {
                    let offset = cx.op(SymExprOp::Add, offset, right.clone());
                    return Some((base, offset));
                }
                if let Some((base, offset)) = right.storage_layout_key(cx)
                    && !left.contains_keccak()
                {
                    let offset = cx.op(SymExprOp::Add, offset, left.clone());
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

#[derive(Clone)]
pub(crate) struct SymExpr {
    pub(in crate::runtime::expr) kind: HashConsed<SymExprKind>,
}

impl PartialEq for SymExpr {
    fn eq(&self, other: &Self) -> bool {
        self.kind.ptr_eq(&other.kind)
    }
}

impl Eq for SymExpr {}

impl Hash for SymExpr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
    }
}

impl PartialOrd for SymExpr {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SymExpr {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.kind.cmp(&other.kind)
    }
}

impl fmt::Debug for SymExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(in crate::runtime) enum SymExprKind {
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

    pub(crate) fn low_byte(self, cx: &mut SymCx) -> Self {
        let this = cx.intern_expr(self);
        if let Some(word) = this.as_const() {
            return cx.constant(U256::from(word.to::<u8>()));
        }
        let mask = cx.constant(U256::from(0xff));
        cx.op(SymExprOp::And, this, mask)
    }

    pub(crate) fn into_byte_exprs(self, cx: &mut SymCx) -> Vec<Self> {
        SymBytes::word(self).materialize(cx)
    }

    pub(crate) fn into_bytes(self) -> SymBytes {
        SymBytes::word(self)
    }

    pub(crate) fn from_bytes(cx: &mut SymCx, bytes: impl IntoIterator<Item = Self>) -> Self {
        let bytes = bytes.into_iter().collect::<Vec<_>>();
        if let Ok(concrete) = concrete_expr_bytes(&bytes, "symbolic word bytes") {
            let mut word = [0u8; 32];
            for (idx, byte) in concrete.into_iter().take(32).enumerate() {
                word[idx] = byte;
            }
            return cx.constant(U256::from_be_bytes(word));
        }

        if let Some(expr) = word_from_extracted_bytes(&bytes) {
            return expr;
        }

        let mut expr = cx.zero();
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            let shift = (31 - idx) * 8;
            let byte = byte.low_byte(cx);
            let byte = if shift == 0 {
                byte
            } else {
                let shift = cx.constant(U256::from(shift));
                cx.op(SymExprOp::Shl, byte, shift)
            };
            expr = cx.op(SymExprOp::Or, expr, byte);
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
            SymExprKind::Const(value) => cx.bool_constant(value.is_zero()),
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                match Self::bool_word_condition_from_parts(condition, then_expr, else_expr) {
                    Some(condition) => cx.not_bool(condition),
                    None => {
                        let zero = cx.zero();
                        cx.eq(self, zero)
                    }
                }
            }
            _ => {
                let zero = cx.zero();
                cx.eq(self, zero)
            }
        }
    }

    pub(crate) fn nonzero_bool(self, cx: &mut SymCx) -> SymBoolExpr {
        let zero = self.into_zero_bool(cx);
        cx.not_bool(zero)
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

    pub(crate) fn extracted_byte(&self, cx: &mut SymCx, index: usize) -> Self {
        debug_assert!(index < 32);
        let shift = cx.constant(U256::from((31 - index) * 8));
        let shifted = cx.op(SymExprOp::Shr, self.clone(), shift);
        let mask = cx.constant(U256::from(0xff));
        cx.op(SymExprOp::And, shifted, mask)
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

    pub(crate) fn byte_term(&self, cx: &mut SymCx, index: usize) -> Option<Self> {
        debug_assert!(index < 32);

        match self.kind() {
            SymExprKind::Const(value) => {
                Some(cx.constant(U256::from(value.to_be_bytes::<32>()[index])))
            }
            SymExprKind::Var(_)
            | SymExprKind::GasLeft(_)
            | SymExprKind::Keccak { .. }
            | SymExprKind::Hash { .. } => Some(self.extracted_byte(cx, index)),
            SymExprKind::Not(value) => {
                let value = value.byte_term(cx, index)?;
                Some(cx.not(value))
            }
            SymExprKind::Ite(cond, then_expr, else_expr) => {
                let then_expr = then_expr.byte_term(cx, index)?;
                let else_expr = else_expr.byte_term(cx, index)?;
                Some(cx.ite(cond.clone(), then_expr, else_expr))
            }
            SymExprKind::Op(op, left, right) => match op {
                SymExprOp::And => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymExprOp::And,
                    |byte| byte == 0xff,
                    |byte| byte == 0,
                ),
                SymExprOp::Or => Self::binary_byte_term(
                    cx,
                    left,
                    right,
                    index,
                    SymExprOp::Or,
                    |byte| byte == 0,
                    |_| false,
                ),
                SymExprOp::Xor => Self::binary_byte_term(
                    cx,
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
                        return Some(cx.zero());
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let source_index = index + shift / 8;
                    if source_index >= 32 {
                        Some(cx.zero())
                    } else {
                        left.byte_term(cx, source_index)
                    }
                }
                SymExprOp::Shr => {
                    let shift = right.eval()?;
                    if shift >= U256::from(256) {
                        return Some(cx.zero());
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let byte_shift = shift / 8;
                    if index < byte_shift {
                        Some(cx.zero())
                    } else {
                        left.byte_term(cx, index - byte_shift)
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
        cx: &mut SymCx,
        left: &Self,
        right: &Self,
        index: usize,
        op: SymExprOp,
        identity: impl Fn(u8) -> bool,
        absorbing: impl Fn(u8) -> bool,
    ) -> Option<Self> {
        let left = left.byte_term(cx, index)?;
        let right = right.byte_term(cx, index)?;
        match (left.byte_const(), right.byte_const()) {
            (Some(left), _) if absorbing(left) => Some(cx.constant(U256::from(left))),
            (_, Some(right)) if absorbing(right) => Some(cx.constant(U256::from(right))),
            (Some(left), _) if identity(left) => Some(right),
            (_, Some(right)) if identity(right) => Some(left),
            _ => Some(cx.op(op, left, right)),
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

    pub(crate) fn add_const(cx: &mut SymCx, expr: Self, value: U256) -> Self {
        if value.is_zero() {
            return expr;
        }
        match expr.kind() {
            SymExprKind::Const(expr) => cx.constant(expr.wrapping_add(value)),
            _ => {
                let value = cx.constant(value);
                cx.op(SymExprOp::Add, expr, value)
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
                cx.keccak_symbol(name, len, bytes)
            }
            SymExprKind::Hash { name, algorithm, bytes } => {
                let bytes = bytes.iter().cloned().map(|byte| byte.fold(cx, folder)).collect();
                cx.hash_symbol(name, algorithm, bytes)
            }
            SymExprKind::Not(value) => {
                let value = value.fold(cx, folder);
                cx.not(value)
            }
            SymExprKind::Op(op, left, right) => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                cx.op(op, left, right)
            }
            SymExprKind::AddMod { left, right, modulus } => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                let modulus = modulus.fold(cx, folder);
                cx.addmod(left, right, modulus)
            }
            SymExprKind::MulMod { left, right, modulus } => {
                let left = left.fold(cx, folder);
                let right = right.fold(cx, folder);
                let modulus = modulus.fold(cx, folder);
                cx.mulmod(left, right, modulus)
            }
            SymExprKind::Ite(condition, then_expr, else_expr) => {
                let condition = condition.fold_exprs(cx, folder);
                let then_expr = then_expr.fold(cx, folder);
                let else_expr = else_expr.fold(cx, folder);
                cx.ite(condition, then_expr, else_expr)
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
