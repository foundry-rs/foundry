use super::*;

/// Set of symbolic variable names collected from expression trees.
pub(crate) type SymbolicVars = BTreeSet<Arc<str>>;

/// Computes the `keccak_word` symbolic expression helper result.
pub(crate) fn keccak_word(bytes: Vec<SymWord>) -> SymWord {
    let len = bytes.len();
    keccak_word_with_len(bytes, SymWord::Concrete(U256::from(len)))
}

/// Computes the `keccak_word_with_len` symbolic expression helper result.
pub(crate) fn keccak_word_with_len(bytes: Vec<SymWord>, len: SymWord) -> SymWord {
    if bytes.iter().all(|byte| matches!(byte, SymWord::Concrete(_)))
        && let SymWord::Concrete(len) = len
        && len <= U256::from(bytes.len())
    {
        let len = len.to::<usize>();
        let bytes = bytes
            .into_iter()
            .take(len)
            .map(|byte| {
                let SymWord::Concrete(byte) = byte else { unreachable!() };
                byte.to::<u8>()
            })
            .collect::<Vec<_>>();
        return SymWord::Concrete(U256::from_be_bytes(keccak256(bytes).0));
    }

    let len = len.into_expr();
    let exprs = bytes.into_iter().map(SymWord::into_expr).collect::<Vec<_>>();
    SymWord::from_expr(Expr::keccak(
        stable_symbol("keccak", format!("{len:?}:{exprs:?}")),
        len,
        exprs,
    ))
}

/// Returns the `symbolic_hash_word_with_len` symbolic expression helper result.
pub(crate) fn symbolic_hash_word_with_len(
    algorithm: &'static str,
    bytes: Vec<SymWord>,
    len: SymWord,
) -> SymWord {
    let len = len.into_expr();
    let exprs = bytes.into_iter().map(SymWord::into_expr).collect::<Vec<_>>();
    let name = stable_symbol(algorithm, format!("{len:?}:{exprs:?}"));
    let mut identity = Vec::with_capacity(exprs.len() + 1);
    identity.push(len);
    identity.extend(exprs);
    SymWord::from_expr(Expr::hash(name, algorithm, identity))
}

/// Implements the `create2_address_word` symbolic expression helper.
pub(crate) fn create2_address_word(
    state: &mut PathState,
    creator: Address,
    salt: SymWord,
    initcode: &SymCode,
) -> Result<(SymWord, Address), SymbolicError> {
    match (salt, initcode.concrete_bytes("symbolic CREATE2 initcode")) {
        (SymWord::Concrete(salt), Ok(initcode)) => {
            let address = creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode);
            Ok((SymWord::Concrete(address_word(address)), address))
        }
        (salt, Ok(initcode)) => {
            let initcode_hash = keccak256(&initcode);
            let word = symbolic_create2_address_word(
                state,
                format!("{creator:?}"),
                salt.into_expr(),
                format!("{initcode_hash:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (salt, Err(SymbolicError::Unsupported("symbolic CREATE2 initcode"))) => {
            let initcode_bytes =
                initcode.bytes().iter().cloned().map(SymWord::into_expr).collect::<Vec<_>>();
            let word = symbolic_create2_address_word(
                state,
                format!("{creator:?}"),
                salt.into_expr(),
                format!("{initcode_bytes:?}"),
            );
            let address = state.world.symbolic_address_slot(word.clone());
            Ok((word, address))
        }
        (_, Err(err)) => Err(err),
    }
}

/// Computes the `compute_create2_address_word` symbolic expression helper result.
pub(crate) fn compute_create2_address_word(
    state: &mut PathState,
    deployer: SymWord,
    salt: SymWord,
    init_code_hash: SymWord,
) -> Result<SymWord, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let salt_concrete = state.constrained_word(&salt);
    let init_code_hash_concrete = state.constrained_word(&init_code_hash);

    if let (Some(deployer), Some(salt), Some(init_code_hash)) =
        (deployer_concrete, salt_concrete, init_code_hash_concrete)
    {
        let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
        let address = deployer.create2(B256::from(salt.to_be_bytes::<32>()), init_code_hash);
        return Ok(SymWord::Concrete(address_word(address)));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{:?}", deployer.into_expr()));
    let init_code_hash_identity = init_code_hash_concrete
        .map(|init_code_hash| {
            let init_code_hash = B256::from(init_code_hash.to_be_bytes::<32>());
            format!("{init_code_hash:?}")
        })
        .unwrap_or_else(|| format!("{:?}", init_code_hash.into_expr()));

    Ok(symbolic_create2_address_word(
        state,
        deployer_identity,
        salt.into_expr(),
        init_code_hash_identity,
    ))
}

/// Computes the `compute_create_address_word` symbolic expression helper result.
pub(crate) fn compute_create_address_word(
    state: &mut PathState,
    deployer: SymWord,
    nonce: SymWord,
) -> Result<SymWord, SymbolicError> {
    let deployer_concrete = state.constrained_word(&deployer).map(word_to_address);
    let nonce_concrete = state.constrained_word(&nonce);

    if let (Some(deployer), Some(nonce)) = (deployer_concrete, nonce_concrete) {
        if nonce > U256::from(u64::MAX) {
            return Err(SymbolicError::Unsupported("symbolic vm.computeCreateAddress nonce"));
        }
        return Ok(SymWord::Concrete(address_word(deployer.create(nonce.to()))));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{:?}", deployer.into_expr()));
    Ok(symbolic_create_address_word(state, deployer_identity, nonce.into_expr()))
}

/// Returns the `symbolic_create_address_word` symbolic expression helper result.
pub(crate) fn symbolic_create_address_word(
    state: &mut PathState,
    creator_identity: String,
    nonce: Expr,
) -> SymWord {
    let word = SymWord::expr(Expr::var(stable_symbol(
        "create_address",
        format!("{creator_identity}:{nonce:?}"),
    )));
    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ult,
        word.clone().into_expr(),
        Expr::Const(U256::from(1) << 160),
    ));
    word
}

/// Returns the `symbolic_create2_address_word` symbolic expression helper result.
pub(crate) fn symbolic_create2_address_word(
    state: &mut PathState,
    creator_identity: String,
    salt: Expr,
    initcode_identity: String,
) -> SymWord {
    let word = SymWord::expr(Expr::var(stable_symbol(
        "create2_address",
        format!("{creator_identity}:{salt:?}:{initcode_identity}"),
    )));
    state.constraints.push(BoolExpr::cmp(
        BoolExprOp::Ult,
        word.clone().into_expr(),
        Expr::Const(U256::from(1) << 160),
    ));
    word
}

/// Returns the `read_storage_writes` symbolic expression helper result.
pub(crate) fn read_storage_writes(
    writes: &[StorageWrite],
    address: Address,
    key: SymWord,
    base: SymWord,
) -> SymWord {
    let mut value = base;
    for write in writes.iter().filter(|write| write.address == address) {
        value = storage_select(key.clone(), write.key.clone(), write.value.clone(), value);
    }
    value
}

/// Implements the `storage_select` symbolic expression helper.
pub(crate) fn storage_select(
    read_key: SymWord,
    write_key: SymWord,
    write_value: SymWord,
    base: SymWord,
) -> SymWord {
    if write_value == base {
        return base;
    }
    let condition = storage_key_eq(read_key, write_key);
    match condition {
        BoolExpr::Const(true) => write_value,
        BoolExpr::Const(false) => base,
        condition => {
            SymWord::from_expr(Expr::ite(condition, write_value.into_expr(), base.into_expr()))
        }
    }
}

/// Implements the `storage_key_eq` symbolic expression helper.
pub(crate) fn storage_key_eq(read_key: SymWord, write_key: SymWord) -> BoolExpr {
    let read_key = read_key.into_expr();
    let write_key = write_key.into_expr();
    if let (Some(read_root), Some(write_root)) =
        (storage_mapping_root_slot(&read_key), storage_mapping_root_slot(&write_key))
        && read_root != write_root
    {
        return BoolExpr::Const(false);
    }
    match (storage_layout_key(&read_key), storage_layout_key(&write_key)) {
        (Some((read_base, read_offset)), Some((write_base, write_offset))) => BoolExpr::and(vec![
            BoolExpr::eq(read_base, write_base),
            BoolExpr::eq(read_offset, write_offset),
        ]),
        (Some(_), None) if matches!(write_key, Expr::Const(_)) => BoolExpr::Const(false),
        (None, Some(_)) if matches!(read_key, Expr::Const(_)) => BoolExpr::Const(false),
        _ => BoolExpr::eq(read_key, write_key),
    }
}

/// Returns the root Solidity storage slot for a mapping-style keccak key.
pub(crate) fn storage_mapping_root_slot(key: &Expr) -> Option<U256> {
    let Expr::Keccak(hash) = key else { return None };
    if !matches!(hash.len.as_ref(), Expr::Const(value) if *value == U256::from(64))
        || hash.bytes.len() < 64
    {
        return None;
    }

    let slot = word_from_bytes(hash.bytes[32..64].iter().cloned().map(|expr| match expr {
        Expr::Const(value) => SymWord::Concrete(value),
        expr => SymWord::from_expr(expr),
    }))
    .into_expr();
    match slot {
        Expr::Const(slot) => Some(slot),
        Expr::Keccak(_) => storage_mapping_root_slot(&slot),
        _ => None,
    }
}

/// Implements the `storage_layout_key` symbolic expression helper.
pub(crate) fn storage_layout_key(key: &Expr) -> Option<(Expr, Expr)> {
    match key {
        Expr::Keccak(_) => Some((key.clone(), Expr::Const(U256::ZERO))),
        Expr::Op(ExprOp::Add, left, right) => {
            if let Some((base, offset)) = storage_layout_key(left)
                && !expr_contains_keccak(right)
            {
                return Some((base, expr_add(offset, (**right).clone())));
            }
            if let Some((base, offset)) = storage_layout_key(right)
                && !expr_contains_keccak(left)
            {
                return Some((base, expr_add(offset, (**left).clone())));
            }
            None
        }
        _ => None,
    }
}

/// Returns the `expr_add` symbolic expression helper result.
pub(crate) fn expr_add(left: Expr, right: Expr) -> Expr {
    match (left, right) {
        (Expr::Const(left), Expr::Const(right)) => Expr::Const(left.wrapping_add(right)),
        (Expr::Const(value), expr) | (expr, Expr::Const(value)) if value.is_zero() => expr,
        (left, right) => Expr::op(ExprOp::Add, left, right),
    }
}

/// Implements the `sym_add` symbolic expression helper.
pub(crate) fn sym_add(left: SymWord, right: SymWord) -> SymWord {
    match (left, right) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            SymWord::Concrete(left.wrapping_add(right))
        }
        (left, right) => SymWord::from_expr(expr_add(left.into_expr(), right.into_expr())),
    }
}

/// Implements the `sym_sub` symbolic expression helper.
pub(crate) fn sym_sub(left: SymWord, right: SymWord) -> SymWord {
    match (left, right) {
        (SymWord::Concrete(left), SymWord::Concrete(right)) => {
            SymWord::Concrete(left.wrapping_sub(right))
        }
        (left, right) => {
            SymWord::from_expr(Expr::op(ExprOp::Sub, left.into_expr(), right.into_expr()))
        }
    }
}

/// Computes the exact EVM `ADDMOD` semantics without truncating the intermediate sum.
pub(crate) fn addmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    if modulus.is_zero() {
        return U256::ZERO;
    }
    u256_from_u512((U512::from(left) + U512::from(right)) % U512::from(modulus))
}

/// Computes the exact EVM `MULMOD` semantics without truncating the intermediate product.
pub(crate) fn mulmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    if modulus.is_zero() {
        return U256::ZERO;
    }
    u256_from_u512((U512::from(left) * U512::from(right)) % U512::from(modulus))
}

/// Converts a known 256-bit-range `U512` result back into `U256`.
fn u256_from_u512(value: U512) -> U256 {
    let limbs = value.as_limbs();
    debug_assert!(limbs[4..].iter().all(|limb| *limb == 0));
    U256::from_limbs([limbs[0], limbs[1], limbs[2], limbs[3]])
}

/// Returns the `expr_contains_keccak` symbolic expression helper result.
pub(crate) fn expr_contains_keccak(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| contains |= matches!(expr, Expr::Keccak(_)));
    contains
}

/// Returns whether a word expression depends on the opaque `GAS` / `gasleft()` value.
pub(crate) fn expr_contains_gasleft(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| contains |= matches!(expr, Expr::GasLeft(_)));
    contains
}

/// Returns the `bool_forces_expr_const_with_context` symbolic expression helper result.
pub(crate) fn bool_forces_expr_const_with_context(
    condition: &BoolExpr,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    match condition {
        BoolExpr::Eq(left, right) => match (left.as_ref(), right.as_ref()) {
            (left, Expr::Const(value)) => expr_equality_forces_const(left, *value, expr, context),
            (Expr::Const(value), right) => expr_equality_forces_const(right, *value, expr, context),
            _ => None,
        },
        BoolExpr::Not(value) => match value.as_ref() {
            BoolExpr::Eq(left, right) => match (left.as_ref(), right.as_ref()) {
                (left, Expr::Const(value)) if value.is_zero() => {
                    expr_nonzero_forces_const(left, expr, context)
                }
                (Expr::Const(value), right) if value.is_zero() => {
                    expr_nonzero_forces_const(right, expr, context)
                }
                _ => None,
            },
            BoolExpr::Not(value) => bool_forces_expr_const_with_context(value, expr, context),
            _ => None,
        },
        BoolExpr::And(values) => values
            .iter()
            .find_map(|value| bool_forces_expr_const_with_context(value, expr, context)),
        _ => None,
    }
}

/// Returns the `expr_equality_forces_const` symbolic expression helper result.
pub(crate) fn expr_equality_forces_const(
    candidate: &Expr,
    value: U256,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    if candidate == expr {
        return Some(value);
    }
    let mask = masked_expr_matches(candidate, expr)?;
    if value & !mask != U256::ZERO || !context_forces_masked_expr(context, expr, mask) {
        return None;
    }
    Some(value)
}

/// Returns the `expr_nonzero_forces_const` symbolic expression helper result.
pub(crate) fn expr_nonzero_forces_const(
    expr: &Expr,
    target: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    match expr {
        Expr::Const(_)
        | Expr::Var(_)
        | Expr::GasLeft(_)
        | Expr::Keccak(_)
        | Expr::Hash(_)
        | Expr::Not(_) => None,
        Expr::Ite(cond, then_expr, else_expr) => {
            if expr_const_value(then_expr).is_some_and(|value| !value.is_zero())
                && expr_const_value(else_expr).is_some_and(|value| value.is_zero())
            {
                bool_forces_expr_const_with_context(cond, target, context)
            } else {
                None
            }
        }
        Expr::Op(ExprOp::Or, left, right) => {
            if expr_const_value(left).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        Expr::Op(ExprOp::And, left, right) => {
            if expr_const_value(left).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        Expr::Op(ExprOp::Shl | ExprOp::Shr, value, shift)
            if expr_const_value(shift).is_some_and(|shift| shift.is_zero()) =>
        {
            expr_nonzero_forces_const(value, target, context)
        }
        Expr::AddMod { .. } | Expr::MulMod { .. } => None,
        Expr::Op(_, _, _) => None,
    }
}

/// Returns whether `masked_expr_matches` holds.
pub(crate) fn masked_expr_matches(candidate: &Expr, target: &Expr) -> Option<U256> {
    match candidate {
        Expr::Op(ExprOp::And, left, right) if left.as_ref() == target => expr_const_value(right),
        Expr::Op(ExprOp::And, left, right) if right.as_ref() == target => expr_const_value(left),
        _ => None,
    }
}

/// Implements the `context_forces_masked_expr` symbolic expression helper.
pub(crate) fn context_forces_masked_expr(context: &[BoolExpr], target: &Expr, mask: U256) -> bool {
    context.iter().any(|condition| match condition {
        BoolExpr::Eq(left, right) => {
            (left.as_ref() == target && masked_expr_matches(right, target) == Some(mask))
                || (right.as_ref() == target && masked_expr_matches(left, target) == Some(mask))
        }
        BoolExpr::And(values) => context_forces_masked_expr(values, target, mask),
        _ => false,
    })
}

/// Returns the `expr_const_value` symbolic expression helper result.
pub(crate) fn expr_const_value(expr: &Expr) -> Option<U256> {
    match expr {
        Expr::Const(value) => Some(*value),
        Expr::Var(_) | Expr::GasLeft(_) | Expr::Keccak(_) | Expr::Hash(_) => None,
        Expr::Not(value) => Some(!expr_const_value(value)?),
        Expr::Op(op, left, right) => {
            Some(eval_expr_op(*op, expr_const_value(left)?, expr_const_value(right)?))
        }
        Expr::AddMod { left, right, modulus } => Some(addmod_word(
            expr_const_value(left)?,
            expr_const_value(right)?,
            expr_const_value(modulus)?,
        )),
        Expr::MulMod { left, right, modulus } => Some(mulmod_word(
            expr_const_value(left)?,
            expr_const_value(right)?,
            expr_const_value(modulus)?,
        )),
        Expr::Ite(cond, then_expr, else_expr) => {
            if bool_const_value(cond)? {
                expr_const_value(then_expr)
            } else {
                expr_const_value(else_expr)
            }
        }
    }
}

/// Returns the `bool_const_value` symbolic expression helper result.
pub(crate) fn bool_const_value(expr: &BoolExpr) -> Option<bool> {
    match expr {
        BoolExpr::Const(value) => Some(*value),
        BoolExpr::Not(value) => Some(!bool_const_value(value)?),
        BoolExpr::And(values) => {
            let mut all_true = true;
            for value in values {
                all_true &= bool_const_value(value)?;
            }
            Some(all_true)
        }
        BoolExpr::Eq(left, right) => Some(expr_const_value(left)? == expr_const_value(right)?),
        BoolExpr::Cmp(op, left, right) => {
            let left = expr_const_value(left)?;
            let right = expr_const_value(right)?;
            Some(match op {
                BoolExprOp::Ult => left < right,
                BoolExprOp::Ugt => left > right,
                BoolExprOp::Ule => left <= right,
                BoolExprOp::Uge => left >= right,
                BoolExprOp::Slt => slt(left, right),
                BoolExprOp::Sgt => slt(right, left),
            })
        }
    }
}

/// Returns the `bool_contains_keccak` symbolic expression helper result.
pub(crate) fn bool_contains_keccak(expr: &BoolExpr) -> bool {
    let mut contains = false;
    expr.visit_exprs(&mut |expr| contains |= matches!(expr, Expr::Keccak(_)));
    contains
}

/// Returns whether a boolean expression depends on the opaque `GAS` / `gasleft()` value.
pub(crate) fn bool_contains_gasleft(expr: &BoolExpr) -> bool {
    let mut contains = false;
    expr.visit_exprs(&mut |expr| contains |= matches!(expr, Expr::GasLeft(_)));
    contains
}

/// Returns the `word_bytes` symbolic expression helper result.
pub(crate) fn word_bytes(word: SymWord) -> Vec<SymWord> {
    match word {
        SymWord::Concrete(word) => word
            .to_be_bytes::<32>()
            .into_iter()
            .map(|byte| SymWord::Concrete(U256::from(byte)))
            .collect(),
        word => (0..32).map(|idx| byte_word(U256::from(idx), word.clone())).collect(),
    }
}

/// Returns the `word_from_bytes` symbolic expression helper result.
pub(crate) fn word_from_bytes(bytes: impl IntoIterator<Item = SymWord>) -> SymWord {
    let bytes = bytes.into_iter().collect::<Vec<_>>();
    if bytes.iter().all(|byte| matches!(byte, SymWord::Concrete(_))) {
        let mut word = [0u8; 32];
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            let SymWord::Concrete(byte) = byte else { unreachable!() };
            word[idx] = byte.to::<u8>();
        }
        return SymWord::Concrete(U256::from_be_bytes(word));
    }

    if let Some(expr) = word_from_extracted_bytes(&bytes) {
        return SymWord::from_expr(expr);
    }

    let mut expr = Expr::Const(U256::ZERO);
    for (idx, byte) in bytes.into_iter().take(32).enumerate() {
        let shift = (31 - idx) * 8;
        let byte = low_byte(byte).into_expr();
        let byte = if shift == 0 {
            byte
        } else {
            Expr::op(ExprOp::Shl, byte, Expr::Const(U256::from(shift)))
        };
        expr = Expr::op(ExprOp::Or, expr, byte);
    }
    SymWord::from_expr(expr)
}

/// Returns the `word_from_extracted_bytes` symbolic expression helper result.
pub(crate) fn word_from_extracted_bytes(bytes: &[SymWord]) -> Option<Expr> {
    if bytes.len() < 32 {
        return None;
    }

    let source = bytes
        .iter()
        .take(32)
        .enumerate()
        .find_map(|(idx, byte)| extracted_byte_source(byte, idx))?;

    for (idx, byte) in bytes.iter().take(32).enumerate() {
        if let Some(byte_source) = extracted_byte_source(byte, idx) {
            if byte_source != source {
                return None;
            }
            continue;
        }

        let SymWord::Concrete(byte) = byte else { return None };
        if expr_known_byte(&source, idx) != Some(byte.to::<u8>()) {
            return None;
        }
    }
    Some(source)
}

/// Implements the `extracted_byte_source` symbolic expression helper.
pub(crate) fn extracted_byte_source(byte: &SymWord, index: usize) -> Option<Expr> {
    let SymWord::Expr(expr) = byte else { return None };
    let expr = strip_low_byte_mask(expr)?;
    if index == 31 {
        return Some(expr.clone());
    }
    let Expr::Op(ExprOp::Shr, source, shift) = expr else { return None };
    let Expr::Const(shift) = shift.as_ref() else { return None };
    (*shift == U256::from((31 - index) * 8)).then(|| *source.clone())
}

/// Implements the `strip_low_byte_mask` symbolic expression helper.
pub(crate) fn strip_low_byte_mask(expr: &Expr) -> Option<&Expr> {
    match expr {
        Expr::Op(ExprOp::And, left, right) if matches!(right.as_ref(), Expr::Const(mask) if *mask == U256::from(0xff)) => {
            Some(strip_low_byte_mask(left).unwrap_or(left))
        }
        Expr::Op(ExprOp::And, left, right) if matches!(left.as_ref(), Expr::Const(mask) if *mask == U256::from(0xff)) => {
            Some(strip_low_byte_mask(right).unwrap_or(right))
        }
        _ => Some(expr),
    }
}

/// Returns the `low_byte` symbolic expression helper result.
pub(crate) fn low_byte(word: SymWord) -> SymWord {
    match word {
        SymWord::Concrete(word) => SymWord::Concrete(U256::from(word.to::<u8>())),
        word => SymWord::from_expr(Expr::op(
            ExprOp::And,
            word.into_expr(),
            Expr::Const(U256::from(0xff)),
        )),
    }
}

/// Returns the `model_word` symbolic expression helper result.
pub(crate) fn model_word(
    word: &SymWord,
    model: &BTreeMap<String, U256>,
) -> Result<U256, SymbolicError> {
    eval_expr(&word.clone().into_expr(), model)
}

/// Returns the `model_bytes` symbolic expression helper result.
pub(crate) fn model_bytes(
    bytes: &[SymWord],
    model: &BTreeMap<String, U256>,
) -> Result<Vec<u8>, SymbolicError> {
    bytes.iter().map(|byte| Ok(model_word(byte, model)?.to::<u8>())).collect()
}

/// Returns the `concrete_bytes` symbolic expression helper result.
pub(crate) fn concrete_bytes(
    bytes: &[SymWord],
    reason: &'static str,
) -> Result<Vec<u8>, SymbolicError> {
    bytes
        .iter()
        .map(|byte| match byte {
            SymWord::Concrete(value) => Ok(value.to::<u8>()),
            SymWord::Expr(_) => Err(SymbolicError::Unsupported(reason)),
        })
        .collect()
}

/// Implements the `calldata_prefix_condition` symbolic expression helper.
pub(crate) fn calldata_prefix_condition(
    calldata: &[SymWord],
    prefix: &[SymWord],
    _reason: &'static str,
) -> Result<Option<BoolExpr>, SymbolicError> {
    if prefix.len() > calldata.len() {
        return Ok(None);
    }
    let mut conditions = Vec::new();
    for (actual, expected) in calldata.iter().zip(prefix) {
        if actual == expected {
            continue;
        }
        match (actual, expected) {
            (SymWord::Concrete(actual), SymWord::Concrete(expected))
                if actual.to::<u8>() == expected.to::<u8>() => {}
            (SymWord::Concrete(_), SymWord::Concrete(_)) => return Ok(None),
            _ => conditions
                .push(BoolExpr::eq(actual.clone().into_expr(), expected.clone().into_expr())),
        }
    }
    Ok(Some(BoolExpr::and(conditions)))
}

/// Implements the `function_mock_match_condition` symbolic expression helper.
pub(crate) fn function_mock_match_condition(
    mock: &FunctionMock,
    callee: Address,
    calldata: &[SymWord],
    reason: &'static str,
) -> Result<Option<BoolExpr>, SymbolicError> {
    let Some(data_condition) = calldata_prefix_condition(calldata, &mock.data, reason)? else {
        return Ok(None);
    };
    Ok(Some(BoolExpr::and(vec![address_match_condition(&mock.callee, callee), data_condition])))
}

/// Returns the `eval_expr` symbolic expression helper result.
pub(crate) fn eval_expr(
    expr: &Expr,
    model: &BTreeMap<String, U256>,
) -> Result<U256, SymbolicError> {
    Ok(match expr {
        Expr::Const(value) => *value,
        Expr::Var(var) => model.get(var.as_ref()).copied().unwrap_or_default(),
        Expr::GasLeft(_) => return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled")),
        Expr::Keccak(hash) => eval_keccak_expr(&hash.len, &hash.bytes, model)?,
        Expr::Hash(hash) => model.get(hash.name.as_ref()).copied().unwrap_or_default(),
        Expr::Not(value) => !eval_expr(value, model)?,
        Expr::Op(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            eval_expr_op(*op, left, right)
        }
        Expr::AddMod { left, right, modulus } => addmod_word(
            eval_expr(left, model)?,
            eval_expr(right, model)?,
            eval_expr(modulus, model)?,
        ),
        Expr::MulMod { left, right, modulus } => mulmod_word(
            eval_expr(left, model)?,
            eval_expr(right, model)?,
            eval_expr(modulus, model)?,
        ),
        Expr::Ite(cond, then_expr, else_expr) => {
            if eval_bool_expr(cond, model)? {
                eval_expr(then_expr, model)?
            } else {
                eval_expr(else_expr, model)?
            }
        }
    })
}

/// Returns the concrete keccak value implied by a solver model.
pub(crate) fn eval_keccak_expr(
    len: &Expr,
    bytes: &[Expr],
    model: &BTreeMap<String, U256>,
) -> Result<U256, SymbolicError> {
    let len = eval_expr(len, model)?;
    if len > U256::from(bytes.len()) {
        return Err(SymbolicError::Solver(
            "solver model uses an invalid keccak length".to_string(),
        ));
    }

    let mut input = Vec::with_capacity(len.to::<usize>());
    for byte in bytes.iter().take(len.to::<usize>()) {
        input.push((eval_expr(byte, model)? & U256::from(0xff)).to::<u8>());
    }

    Ok(U256::from_be_bytes(keccak256(input).0))
}

/// Returns the `eval_expr_op` symbolic expression helper result.
pub(crate) fn eval_expr_op(op: ExprOp, left: U256, right: U256) -> U256 {
    match op {
        ExprOp::Add => left.wrapping_add(right),
        ExprOp::Sub => left.wrapping_sub(right),
        ExprOp::Mul => left.wrapping_mul(right),
        ExprOp::UDiv => {
            if right.is_zero() {
                U256::ZERO
            } else {
                left / right
            }
        }
        ExprOp::URem => {
            if right.is_zero() {
                U256::ZERO
            } else {
                left % right
            }
        }
        ExprOp::SDiv => sdiv(left, right),
        ExprOp::SRem => smod(left, right),
        ExprOp::And => left & right,
        ExprOp::Or => left | right,
        ExprOp::Xor => left ^ right,
        ExprOp::Shl => {
            if right >= U256::from(256) {
                U256::ZERO
            } else {
                left << right.to::<usize>()
            }
        }
        ExprOp::Shr => {
            if right >= U256::from(256) {
                U256::ZERO
            } else {
                left >> right.to::<usize>()
            }
        }
        ExprOp::Sar => {
            if right >= U256::from(256) {
                sar(left, 256)
            } else {
                sar(left, right.to::<usize>())
            }
        }
    }
}

/// Returns the `eval_bool_expr` symbolic expression helper result.
pub(crate) fn eval_bool_expr(
    expr: &BoolExpr,
    model: &BTreeMap<String, U256>,
) -> Result<bool, SymbolicError> {
    Ok(match expr {
        BoolExpr::Const(value) => *value,
        BoolExpr::Not(value) => !eval_bool_expr(value, model)?,
        BoolExpr::And(values) => {
            for value in values {
                if !eval_bool_expr(value, model)? {
                    return Ok(false);
                }
            }
            true
        }
        BoolExpr::Eq(left, right) => eval_expr(left, model)? == eval_expr(right, model)?,
        BoolExpr::Cmp(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            match op {
                BoolExprOp::Ult => left < right,
                BoolExprOp::Ugt => left > right,
                BoolExprOp::Ule => left <= right,
                BoolExprOp::Uge => left >= right,
                BoolExprOp::Slt => slt(left, right),
                BoolExprOp::Sgt => slt(right, left),
            }
        }
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum SymWord {
    Concrete(U256),
    Expr(Expr),
}

impl SymWord {
    /// Implements the `zero` symbolic expression helper.
    pub(crate) const fn zero() -> Self {
        Self::Concrete(U256::ZERO)
    }

    /// Converts an expression into a symbolic word, preserving the concrete fast path.
    pub(crate) fn from_expr(expr: Expr) -> Self {
        match expr {
            Expr::Const(value) => Self::Concrete(value),
            expr => Self::Expr(expr),
        }
    }

    /// Builds a symbolic word from an expression.
    pub(crate) fn expr(expr: Expr) -> Self {
        Self::from_expr(expr)
    }

    /// Returns whether this word depends on the opaque `GAS` / `gasleft()` value.
    pub(crate) fn contains_gasleft(&self) -> bool {
        match self {
            Self::Concrete(_) => false,
            Self::Expr(expr) => expr_contains_gasleft(expr),
        }
    }

    /// Returns whether this word is exactly the opaque `GAS` / `gasleft()` value.
    pub(crate) const fn is_raw_gasleft(&self) -> bool {
        matches!(self, Self::Expr(Expr::GasLeft(_)))
    }

    /// Implements the `into_expr` symbolic expression helper.
    pub(crate) fn into_expr(self) -> Expr {
        match self {
            Self::Concrete(value) => Expr::Const(value),
            Self::Expr(expr) => expr,
        }
    }

    /// Converts values for the `from_bool` symbolic expression helper.
    pub(crate) fn from_bool(value: BoolExpr) -> Self {
        match value {
            BoolExpr::Const(value) => Self::Concrete(U256::from(value)),
            value => Self::from_expr(Expr::ite(
                value,
                Expr::Const(U256::from(1)),
                Expr::Const(U256::ZERO),
            )),
        }
    }

    /// Implements the `truth` symbolic expression helper.
    pub(crate) fn truth(&self) -> Option<bool> {
        match self {
            Self::Concrete(value) => Some(!value.is_zero()),
            _ => None,
        }
    }

    /// Implements the `into_zero_bool` symbolic expression helper.
    pub(crate) fn into_zero_bool(self) -> BoolExpr {
        match self {
            Self::Concrete(value) => BoolExpr::Const(value.is_zero()),
            Self::Expr(Expr::Ite(cond, then_expr, else_expr))
                if then_expr.as_ref() == &Expr::Const(U256::from(1))
                    && else_expr.as_ref() == &Expr::Const(U256::ZERO) =>
            {
                cond.not()
            }
            Self::Expr(Expr::Ite(cond, then_expr, else_expr))
                if then_expr.as_ref() == &Expr::Const(U256::ZERO)
                    && else_expr.as_ref() == &Expr::Const(U256::from(1)) =>
            {
                *cond
            }
            value => BoolExpr::eq(value.into_expr(), Expr::Const(U256::ZERO)),
        }
    }

    /// Implements the `nonzero_bool` symbolic expression helper.
    pub(crate) fn nonzero_bool(self) -> BoolExpr {
        self.into_zero_bool().not()
    }

    /// Implements the `into_concrete` symbolic expression helper.
    pub(crate) fn into_concrete(self, reason: &'static str) -> Result<U256, SymbolicError> {
        match self {
            Self::Concrete(value) => Ok(value),
            Self::Expr(_) => Err(SymbolicError::Unsupported(reason)),
        }
    }

    /// Implements the `into_usize` symbolic expression helper.
    pub(crate) fn into_usize(self, reason: &'static str) -> Result<usize, SymbolicError> {
        let value = self.into_concrete(reason)?;
        if value > U256::from(usize::MAX) {
            return Err(SymbolicError::Unsupported(reason));
        }
        Ok(value.to::<usize>())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum Expr {
    Const(U256),
    Var(Arc<str>),
    GasLeft(usize),
    Keccak(Box<KeccakExpr>),
    Hash(Box<HashExpr>),
    Not(Box<Self>),
    Op(ExprOp, Box<Self>, Box<Self>),
    AddMod { left: Box<Self>, right: Box<Self>, modulus: Box<Self> },
    MulMod { left: Box<Self>, right: Box<Self>, modulus: Box<Self> },
    Ite(Box<BoolExpr>, Box<Self>, Box<Self>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct KeccakExpr {
    pub(crate) name: Arc<str>,
    pub(crate) len: Box<Expr>,
    pub(crate) bytes: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct HashExpr {
    pub(crate) name: Arc<str>,
    pub(crate) algorithm: &'static str,
    pub(crate) bytes: Vec<Expr>,
}

impl Expr {
    /// Builds a symbolic variable expression.
    pub(crate) fn var(name: impl Into<Arc<str>>) -> Self {
        Self::Var(name.into())
    }

    /// Builds a symbolic keccak expression.
    pub(crate) fn keccak(name: impl Into<Arc<str>>, len: Self, bytes: Vec<Self>) -> Self {
        Self::Keccak(Box::new(KeccakExpr { name: name.into(), len: Box::new(len), bytes }))
    }

    /// Builds an opaque symbolic hash expression.
    pub(crate) fn hash(
        name: impl Into<Arc<str>>,
        algorithm: &'static str,
        bytes: Vec<Self>,
    ) -> Self {
        Self::Hash(Box::new(HashExpr { name: name.into(), algorithm, bytes }))
    }

    /// Builds a conditional expression.
    pub(crate) fn ite(cond: BoolExpr, then_expr: Self, else_expr: Self) -> Self {
        match cond {
            BoolExpr::Const(true) => then_expr,
            BoolExpr::Const(false) => else_expr,
            cond => {
                if then_expr == else_expr {
                    then_expr
                } else {
                    Self::Ite(Box::new(cond), Box::new(then_expr), Box::new(else_expr))
                }
            }
        }
    }

    /// Builds a bitwise-not expression.
    pub(crate) fn not(value: Self) -> Self {
        match value {
            Self::Const(value) => Self::Const(!value),
            Self::Not(value) => *value,
            value => Self::Not(Box::new(value)),
        }
    }

    /// Visits this expression and all child expressions.
    pub(crate) fn visit(&self, visitor: &mut impl FnMut(&Self)) {
        visitor(self);
        match self {
            Self::Const(_) | Self::Var(_) | Self::GasLeft(_) => {}
            Self::Keccak(hash) => {
                hash.len.visit(visitor);
                for byte in &hash.bytes {
                    byte.visit(visitor);
                }
            }
            Self::Hash(hash) => {
                for byte in &hash.bytes {
                    byte.visit(visitor);
                }
            }
            Self::Not(value) => value.visit(visitor),
            Self::Op(_, left, right) => {
                left.visit(visitor);
                right.visit(visitor);
            }
            Self::AddMod { left, right, modulus } | Self::MulMod { left, right, modulus } => {
                left.visit(visitor);
                right.visit(visitor);
                modulus.visit(visitor);
            }
            Self::Ite(cond, left, right) => {
                cond.visit_exprs(visitor);
                left.visit(visitor);
                right.visit(visitor);
            }
        }
    }

    /// Implements the `op` symbolic expression helper.
    pub(crate) fn op(op: ExprOp, left: Self, right: Self) -> Self {
        if let (Self::Const(left), Self::Const(right)) = (&left, &right) {
            return Self::Const(eval_expr_op(op, *left, *right));
        }

        match (op, left, right) {
            (ExprOp::Add, Self::Const(value), expr) | (ExprOp::Add, expr, Self::Const(value))
                if value.is_zero() =>
            {
                expr
            }
            (ExprOp::Sub, expr, Self::Const(value)) if value.is_zero() => expr,
            (ExprOp::Sub, left, right) if left == right => Self::Const(U256::ZERO),
            (ExprOp::Mul, Self::Const(value), _) | (ExprOp::Mul, _, Self::Const(value))
                if value.is_zero() =>
            {
                Self::Const(U256::ZERO)
            }
            (ExprOp::Mul, Self::Const(value), expr) | (ExprOp::Mul, expr, Self::Const(value))
                if value == U256::from(1) =>
            {
                expr
            }
            (ExprOp::UDiv | ExprOp::URem | ExprOp::SDiv | ExprOp::SRem, _, Self::Const(value))
                if value.is_zero() =>
            {
                Self::Const(U256::ZERO)
            }
            (ExprOp::UDiv | ExprOp::SDiv, expr, Self::Const(value)) if value == U256::from(1) => {
                expr
            }
            (ExprOp::URem | ExprOp::SRem, _, Self::Const(value)) if value == U256::from(1) => {
                Self::Const(U256::ZERO)
            }
            (ExprOp::And, Self::Const(value), _) | (ExprOp::And, _, Self::Const(value))
                if value.is_zero() =>
            {
                Self::Const(U256::ZERO)
            }
            (ExprOp::And, Self::Const(value), expr) | (ExprOp::And, expr, Self::Const(value))
                if value == U256::MAX =>
            {
                expr
            }
            (ExprOp::And, left, right) if left == right => left,
            (ExprOp::And, Self::Const(mask), expr) | (ExprOp::And, expr, Self::Const(mask)) => {
                Self::and_const(expr, mask)
            }
            (ExprOp::Or | ExprOp::Xor, Self::Const(value), expr)
            | (ExprOp::Or | ExprOp::Xor, expr, Self::Const(value))
                if value.is_zero() =>
            {
                expr
            }
            (ExprOp::Shl | ExprOp::Shr | ExprOp::Sar, expr, Self::Const(value))
                if value.is_zero() =>
            {
                expr
            }
            (ExprOp::Shl | ExprOp::Shr, Self::Const(value), _) if value.is_zero() => {
                Self::Const(U256::ZERO)
            }
            (op, left, right) => Self::Op(op, Box::new(left), Box::new(right)),
        }
    }

    /// Builds an exact EVM `ADDMOD` expression.
    pub(crate) fn addmod(left: Self, right: Self, modulus: Self) -> Self {
        if matches!(modulus, Self::Const(value) if value.is_zero() || value == U256::from(1)) {
            return Self::Const(U256::ZERO);
        }
        if let (Self::Const(left), Self::Const(right), Self::Const(modulus)) =
            (&left, &right, &modulus)
        {
            return Self::Const(addmod_word(*left, *right, *modulus));
        }
        Self::AddMod { left: Box::new(left), right: Box::new(right), modulus: Box::new(modulus) }
    }

    /// Builds an exact EVM `MULMOD` expression.
    pub(crate) fn mulmod(left: Self, right: Self, modulus: Self) -> Self {
        if matches!(modulus, Self::Const(value) if value.is_zero() || value == U256::from(1)) {
            return Self::Const(U256::ZERO);
        }
        if let (Self::Const(left), Self::Const(right), Self::Const(modulus)) =
            (&left, &right, &modulus)
        {
            return Self::Const(mulmod_word(*left, *right, *modulus));
        }
        Self::MulMod { left: Box::new(left), right: Box::new(right), modulus: Box::new(modulus) }
    }

    fn and_const(expr: Self, mask: U256) -> Self {
        if mask.is_zero() {
            return Self::Const(U256::ZERO);
        }
        if mask == U256::MAX {
            return expr;
        }

        match expr {
            Self::Op(ExprOp::And, left, right) => match (*left, *right) {
                (Self::Const(existing), inner) | (inner, Self::Const(existing))
                    if existing == mask =>
                {
                    Self::and_const(inner, mask)
                }
                (left, right) if left == right => Self::and_const(left, mask),
                (left, right) => Self::Op(
                    ExprOp::And,
                    Box::new(Self::Op(ExprOp::And, Box::new(left), Box::new(right))),
                    Box::new(Self::Const(mask)),
                ),
            },
            expr => Self::Op(ExprOp::And, Box::new(expr), Box::new(Self::Const(mask))),
        }
    }

    /// Implements the `collect_vars` symbolic expression helper.
    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        self.visit(&mut |expr| match expr {
            Self::Var(var) => {
                vars.insert(var.clone());
            }
            Self::Keccak(hash) => {
                vars.insert(hash.name.clone());
            }
            Self::Hash(hash) => {
                vars.insert(hash.name.clone());
            }
            Self::Const(_)
            | Self::GasLeft(_)
            | Self::Not(_)
            | Self::Op(_, _, _)
            | Self::AddMod { .. }
            | Self::MulMod { .. }
            | Self::Ite(_, _, _) => {}
        });
    }

    /// Implements the `smt` symbolic expression helper.
    pub(crate) fn smt(&self) -> String {
        match self {
            Self::Const(value) => format!("(_ bv{value} 256)"),
            Self::Var(var) => var.to_string(),
            Self::GasLeft(id) => format!("gasleft_{id}"),
            Self::Keccak(hash) => hash.name.to_string(),
            Self::Hash(hash) => hash.name.to_string(),
            Self::Not(value) => format!("(bvnot {})", value.smt()),
            Self::Op(op, left, right) => format!("({} {} {})", op.smt(), left.smt(), right.smt()),
            Self::AddMod { left, right, modulus } => {
                smt_wide_modular_arithmetic("bvadd", left, right, modulus)
            }
            Self::MulMod { left, right, modulus } => {
                smt_wide_modular_arithmetic("bvmul", left, right, modulus)
            }
            Self::Ite(cond, left, right) => {
                format!("(ite {} {} {})", cond.smt(), left.smt(), right.smt())
            }
        }
    }
}

/// Encodes EVM `ADDMOD`/`MULMOD` by widening operands before modular reduction.
fn smt_wide_modular_arithmetic(
    op: &'static str,
    left: &Expr,
    right: &Expr,
    modulus: &Expr,
) -> String {
    let left = left.smt();
    let right = right.smt();
    let modulus = modulus.smt();
    format!(
        "(ite (= {modulus} (_ bv0 256)) (_ bv0 256) ((_ extract 255 0) (bvurem ({op} ((_ zero_extend 256) {left}) ((_ zero_extend 256) {right})) ((_ zero_extend 256) {modulus}))))"
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum ExprOp {
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

impl ExprOp {
    /// Implements the `smt` symbolic expression helper.
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
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoolExpr {
    Const(bool),
    Not(Box<Self>),
    And(Vec<Self>),
    Eq(Box<Expr>, Box<Expr>),
    Cmp(BoolExprOp, Box<Expr>, Box<Expr>),
}

impl BoolExpr {
    /// Visits this boolean expression and all child boolean expressions.
    pub(crate) fn visit(&self, visitor: &mut impl FnMut(&Self)) {
        visitor(self);
        match self {
            Self::Const(_) | Self::Eq(_, _) | Self::Cmp(_, _, _) => {}
            Self::Not(value) => value.visit(visitor),
            Self::And(values) => {
                for value in values {
                    value.visit(visitor);
                }
            }
        }
    }

    /// Visits all word expressions contained in this boolean expression.
    pub(crate) fn visit_exprs(&self, visitor: &mut impl FnMut(&Expr)) {
        match self {
            Self::Const(_) => {}
            Self::Not(value) => value.visit_exprs(visitor),
            Self::And(values) => {
                for value in values {
                    value.visit_exprs(visitor);
                }
            }
            Self::Eq(left, right) | Self::Cmp(_, left, right) => {
                left.visit(visitor);
                right.visit(visitor);
            }
        }
    }

    /// Implements the `eq` symbolic expression helper.
    pub(crate) fn eq(left: Expr, right: Expr) -> Self {
        if left == right {
            return Self::Const(true);
        }
        match (&left, &right) {
            (Expr::Const(left), Expr::Const(right)) => Self::Const(left == right),
            (left, Expr::Const(right)) => {
                if let Some(left) = expr_known_word(left) {
                    return Self::Const(left == *right);
                }
                Self::Eq(Box::new(left.clone()), Box::new(Expr::Const(*right)))
            }
            (Expr::Const(left), right) => {
                if let Some(right) = expr_known_word(right) {
                    return Self::Const(*left == right);
                }
                Self::Eq(Box::new(Expr::Const(*left)), Box::new(right.clone()))
            }
            (Expr::Keccak(left), Expr::Keccak(right)) if left.bytes.len() == right.bytes.len() => {
                let mut conditions = vec![Self::eq((*left.len).clone(), (*right.len).clone())];
                conditions.extend(
                    left.bytes
                        .iter()
                        .cloned()
                        .zip(right.bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right)),
                );
                Self::and(conditions)
            }
            (Expr::Hash(left), Expr::Hash(right))
                if left.algorithm == right.algorithm && left.bytes.len() == right.bytes.len() =>
            {
                Self::and(
                    left.bytes
                        .iter()
                        .cloned()
                        .zip(right.bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right))
                        .collect(),
                )
            }
            _ => Self::Eq(Box::new(left), Box::new(right)),
        }
    }

    /// Implements the `and` symbolic expression helper.
    pub(crate) fn and(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value {
                Self::Const(true) => {}
                Self::Const(false) => return Self::Const(false),
                Self::And(values) => out.extend(values),
                value => out.push(value),
            }
        }
        if out.is_empty() {
            Self::Const(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::And(out)
        }
    }

    /// Implements the `or` symbolic expression helper.
    pub(crate) fn or(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value {
                Self::Const(false) => {}
                Self::Const(true) => return Self::Const(true),
                value => out.push(value),
            }
        }
        if out.is_empty() {
            Self::Const(false)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::and(out.into_iter().map(Self::not).collect()).not()
        }
    }

    /// Implements the `cmp` symbolic expression helper.
    pub(crate) fn cmp(op: BoolExprOp, left: Expr, right: Expr) -> Self {
        if left == right {
            return Self::Const(matches!(op, BoolExprOp::Ule | BoolExprOp::Uge));
        }
        if let (Expr::Const(left), Expr::Const(right)) = (&left, &right) {
            return Self::Const(match op {
                BoolExprOp::Ult => left < right,
                BoolExprOp::Ugt => left > right,
                BoolExprOp::Ule => left <= right,
                BoolExprOp::Uge => left >= right,
                BoolExprOp::Slt => slt(*left, *right),
                BoolExprOp::Sgt => slt(*right, *left),
            });
        }
        match (op, &left, &right) {
            (BoolExprOp::Ugt, Expr::Const(value), _) if value.is_zero() => {
                return Self::Const(false);
            }
            (BoolExprOp::Ule, Expr::Const(value), _) if value.is_zero() => {
                return Self::Const(true);
            }
            (BoolExprOp::Ult, _, Expr::Const(value)) if value.is_zero() => {
                return Self::Const(false);
            }
            (BoolExprOp::Uge, _, Expr::Const(value)) if value.is_zero() => {
                return Self::Const(true);
            }
            (BoolExprOp::Ult, Expr::Const(value), _) if *value == U256::MAX => {
                return Self::Const(false);
            }
            (BoolExprOp::Uge, Expr::Const(value), _) if *value == U256::MAX => {
                return Self::Const(true);
            }
            (BoolExprOp::Ugt, _, Expr::Const(value)) if *value == U256::MAX => {
                return Self::Const(false);
            }
            (BoolExprOp::Ule, _, Expr::Const(value)) if *value == U256::MAX => {
                return Self::Const(true);
            }
            _ => {}
        }
        Self::Cmp(op, Box::new(left), Box::new(right))
    }

    /// Implements the `not` symbolic expression helper.
    pub(crate) fn not(self) -> Self {
        match self {
            Self::Const(value) => Self::Const(!value),
            Self::Not(value) => *value,
            Self::And(values) => Self::Not(Box::new(Self::And(values))),
            value => Self::Not(Box::new(value)),
        }
    }

    /// Implements the `collect_vars` symbolic expression helper.
    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        self.visit(&mut |expr| match expr {
            Self::Eq(left, right) | Self::Cmp(_, left, right) => {
                left.collect_vars(vars);
                right.collect_vars(vars);
            }
            Self::Const(_) | Self::Not(_) | Self::And(_) => {}
        });
    }

    /// Implements the `smt` symbolic expression helper.
    pub(crate) fn smt(&self) -> String {
        match self {
            Self::Const(value) => value.to_string(),
            Self::Not(value) => format!("(not {})", value.smt()),
            Self::And(values) => {
                format!("(and {})", values.iter().map(Self::smt).collect::<Vec<_>>().join(" "))
            }
            Self::Eq(left, right) => format!("(= {} {})", left.smt(), right.smt()),
            Self::Cmp(op, left, right) => format!("({} {} {})", op.smt(), left.smt(), right.smt()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoolExprOp {
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl BoolExprOp {
    /// Implements the `smt` symbolic expression helper.
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
}

/// Returns the `u256_to_usize` symbolic expression helper result.
pub(crate) fn u256_to_usize(value: U256) -> Option<usize> {
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

/// Returns the `bool_upper_bound_usize` symbolic expression helper result.
pub(crate) fn bool_upper_bound_usize(condition: &BoolExpr, expr: &Expr) -> Option<usize> {
    match condition {
        BoolExpr::Const(_) | BoolExpr::Not(_) => None,
        BoolExpr::And(values) => {
            let mut bound: Option<usize> = None;
            for value in values {
                if let Some(candidate) = bool_upper_bound_usize(value, expr) {
                    bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                }
            }
            bound
        }
        BoolExpr::Eq(left, right) => match (left.as_ref() == expr, right.as_ref() == expr) {
            (true, _) => expr_const_value(right).and_then(u256_to_usize),
            (_, true) => expr_const_value(left).and_then(u256_to_usize),
            _ => None,
        },
        BoolExpr::Cmp(op, left, right) => {
            if left.as_ref() == expr {
                match op {
                    BoolExprOp::Ult => expr_const_value(right)
                        .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                        .and_then(u256_to_usize),
                    BoolExprOp::Ule => expr_const_value(right).and_then(u256_to_usize),
                    _ => None,
                }
            } else if right.as_ref() == expr {
                match op {
                    BoolExprOp::Ugt => expr_const_value(left)
                        .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                        .and_then(u256_to_usize),
                    BoolExprOp::Uge => expr_const_value(left).and_then(u256_to_usize),
                    _ => None,
                }
            } else {
                None
            }
        }
    }
}
