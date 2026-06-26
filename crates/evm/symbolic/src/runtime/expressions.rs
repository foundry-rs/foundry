use super::*;

pub(crate) type SymbolicVars = IndexSet<Symbol>;

pub(crate) type SymbolicModel = HashMap<Symbol, U256>;

pub(crate) trait SymbolicModelLookup {
    fn value(&self, name: Symbol) -> Option<U256>;

    fn contains_name(&self, name: Symbol) -> bool {
        self.value(name).is_some()
    }
}

impl SymbolicModelLookup for SymbolicModel {
    fn value(&self, name: Symbol) -> Option<U256> {
        self.get(&name).copied()
    }
}

#[cfg(test)]
impl SymbolicModelLookup for BTreeMap<String, U256> {
    fn value(&self, name: Symbol) -> Option<U256> {
        self.get(name.as_str()).copied()
    }
}

type SymbolInterner = inturn::Interner<Symbol, DefaultHashBuilder>;

static SYMBOL_INTERNER: LazyLock<SymbolInterner> =
    LazyLock::new(|| SymbolInterner::with_capacity_and_hasher(1024, DefaultHashBuilder::default()));

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Symbol(NonZeroU32);

impl Symbol {
    pub(crate) fn intern(name: impl AsRef<str>) -> Self {
        SYMBOL_INTERNER.intern(name.as_ref())
    }

    pub(crate) fn as_str(self) -> &'static str {
        SYMBOL_INTERNER.resolve(self)
    }
}

impl inturn::InternerSymbol for Symbol {
    fn try_from_usize(id: usize) -> Option<Self> {
        let id = u32::try_from(id).ok()?.checked_add(1)?;
        NonZeroU32::new(id).map(Self)
    }

    fn to_usize(self) -> usize {
        self.0.get() as usize - 1
    }
}

impl fmt::Debug for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for Symbol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub(crate) fn keccak_word(bytes: Vec<SymWord>) -> SymWord {
    let len = bytes.len();
    keccak_word_with_len(bytes, SymWord::constant(U256::from(len)))
}

pub(crate) fn keccak_word_with_len(bytes: Vec<SymWord>, len: SymWord) -> SymWord {
    if bytes.iter().all(|byte| byte.as_const().is_some())
        && let Some(len) = len.as_const()
        && len <= U256::from(bytes.len())
    {
        let len = len.to::<usize>();
        let bytes = bytes
            .into_iter()
            .take(len)
            .map(|byte| byte.as_const().expect("checked concrete byte").to::<u8>())
            .collect::<Vec<_>>();
        return SymWord::constant(U256::from_be_bytes(keccak256(bytes).0));
    }

    let len = len.into_expr();
    let exprs = bytes.into_iter().map(SymWord::into_expr).collect::<Vec<_>>();
    SymWord::expr(Expr::keccak(stable_symbol("keccak", format!("{len:?}:{exprs:?}")), len, exprs))
}

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
    SymWord::expr(Expr::hash(name, algorithm, identity))
}

pub(crate) fn create2_address_word(
    state: &mut PathState,
    creator: Address,
    salt: SymWord,
    initcode: &SymCode,
) -> Result<(SymWord, Address), SymbolicError> {
    match (salt.as_const(), initcode.concrete_bytes("symbolic CREATE2 initcode")) {
        (Some(salt), Ok(initcode)) => {
            let address = creator.create2_from_code(salt.to_be_bytes::<32>(), &initcode);
            Ok((SymWord::constant(address_word(address)), address))
        }
        (None, Ok(initcode)) => {
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
        (_, Err(SymbolicError::Unsupported("symbolic CREATE2 initcode"))) => {
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
        return Ok(SymWord::constant(address_word(address)));
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
        return Ok(SymWord::constant(address_word(deployer.create(nonce.to()))));
    }

    let deployer_identity = deployer_concrete
        .map(|deployer| format!("{deployer:?}"))
        .unwrap_or_else(|| format!("{:?}", deployer.into_expr()));
    Ok(symbolic_create_address_word(state, deployer_identity, nonce.into_expr()))
}

pub(crate) fn symbolic_create_address_word(
    state: &mut PathState,
    creator_identity: String,
    nonce: Expr,
) -> SymWord {
    let word = SymWord::expr(Expr::var(stable_symbol(
        "create_address",
        format!("{creator_identity}:{nonce:?}"),
    )));
    state.constraints.push(BoolExpr::cmp_word_const(BoolExprOp::Ult, &word, U256::from(1) << 160));
    word
}

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
    state.constraints.push(BoolExpr::cmp_word_const(BoolExprOp::Ult, &word, U256::from(1) << 160));
    word
}

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
        condition => SymWord::expr(Expr::ite(condition, write_value.into_expr(), base.into_expr())),
    }
}

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
        (Some(_), None) if write_key.as_const().is_some() => BoolExpr::Const(false),
        (None, Some(_)) if read_key.as_const().is_some() => BoolExpr::Const(false),
        _ => BoolExpr::eq(read_key, write_key),
    }
}

/// Returns the root Solidity storage slot for a mapping-style keccak key.
pub(crate) fn storage_mapping_root_slot(key: &Expr) -> Option<U256> {
    let ExprInner::Keccak(hash) = key.as_inner() else { return None };
    if hash.len.as_const() != Some(U256::from(64)) || hash.bytes.len() < 64 {
        return None;
    }

    let slot = word_from_bytes(hash.bytes[32..64].iter().cloned().map(SymWord::expr)).into_expr();
    match slot.as_inner() {
        ExprInner::Const(slot) => Some(*slot),
        ExprInner::Keccak(_) => storage_mapping_root_slot(&slot),
        _ => None,
    }
}

pub(crate) fn storage_layout_key(key: &Expr) -> Option<(Expr, Expr)> {
    match key.as_inner() {
        ExprInner::Keccak(_) => Some((key.clone(), Expr::constant(U256::ZERO))),
        ExprInner::Op(ExprOp::Add, left, right) => {
            if let Some((base, offset)) = storage_layout_key(left)
                && !expr_contains_keccak(right)
            {
                return Some((base, expr_add(offset, right.clone())));
            }
            if let Some((base, offset)) = storage_layout_key(right)
                && !expr_contains_keccak(left)
            {
                return Some((base, expr_add(offset, left.clone())));
            }
            None
        }
        _ => None,
    }
}

pub(crate) fn expr_add(left: Expr, right: Expr) -> Expr {
    if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
        return Expr::constant(left_value.wrapping_add(right_value));
    }
    match (left.as_const(), right.as_const()) {
        (Some(value), _) if value.is_zero() => right,
        (_, Some(value)) if value.is_zero() => left,
        _ => Expr::op(ExprOp::Add, left, right),
    }
}

pub(crate) fn sym_add(left: SymWord, right: SymWord) -> SymWord {
    if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
        return SymWord::constant(left_value.wrapping_add(right_value));
    }
    SymWord::expr(expr_add(left.into_expr(), right.into_expr()))
}

pub(crate) fn sym_sub(left: SymWord, right: SymWord) -> SymWord {
    if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
        return SymWord::constant(left_value.wrapping_sub(right_value));
    }
    SymWord::expr(Expr::op(ExprOp::Sub, left.into_expr(), right.into_expr()))
}

/// Computes the exact EVM `ADDMOD` semantics without truncating the intermediate sum.
pub(crate) fn addmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    left.add_mod(right, modulus)
}

/// Computes the exact EVM `MULMOD` semantics without truncating the intermediate product.
pub(crate) fn mulmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    left.mul_mod(right, modulus)
}

pub(crate) fn expr_contains_keccak(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| contains |= matches!(expr.as_inner(), ExprInner::Keccak(_)));
    contains
}

/// Returns whether a word expression depends on the opaque `GAS` / `gasleft()` value.
pub(crate) fn expr_contains_gasleft(expr: &Expr) -> bool {
    let mut contains = false;
    expr.visit(&mut |expr| contains |= matches!(expr.as_inner(), ExprInner::GasLeft(_)));
    contains
}

pub(crate) fn bool_forces_expr_const_with_context(
    condition: &BoolExpr,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    match condition.as_inner() {
        BoolExprRef::Eq(left, right) => match (left.as_inner(), right.as_inner()) {
            (_, ExprInner::Const(value)) => expr_equality_forces_const(left, *value, expr, context),
            (ExprInner::Const(value), _) => {
                expr_equality_forces_const(right, *value, expr, context)
            }
            _ => None,
        },
        BoolExprRef::Not(value) => match value.as_inner() {
            BoolExprRef::Eq(left, right) => match (left.as_inner(), right.as_inner()) {
                (left, ExprInner::Const(value)) if value.is_zero() => {
                    expr_nonzero_forces_const_inner(left, expr, context)
                }
                (ExprInner::Const(value), right) if value.is_zero() => {
                    expr_nonzero_forces_const_inner(right, expr, context)
                }
                _ => None,
            },
            BoolExprRef::Not(value) => bool_forces_expr_const_with_context(value, expr, context),
            _ => None,
        },
        BoolExprRef::And(values) => values
            .iter()
            .find_map(|value| bool_forces_expr_const_with_context(value, expr, context)),
        _ => None,
    }
}

pub(crate) fn expr_equality_forces_const(
    candidate: &Expr,
    value: U256,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    if candidate == expr {
        return Some(value);
    }
    expr_equality_forces_const_inner(candidate.as_inner(), value, expr, context)
}

fn expr_equality_forces_const_inner(
    candidate: &ExprInner,
    value: U256,
    expr: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    let mask = masked_expr_matches(candidate, expr)?;
    if value & !mask != U256::ZERO || !context_forces_masked_expr(context, expr, mask) {
        return None;
    }
    Some(value)
}

pub(crate) fn expr_nonzero_forces_const(
    expr: &Expr,
    target: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    expr_nonzero_forces_const_inner(expr.as_inner(), target, context)
}

fn expr_nonzero_forces_const_inner(
    expr: &ExprInner,
    target: &Expr,
    context: &[BoolExpr],
) -> Option<U256> {
    match expr {
        ExprInner::Const(_)
        | ExprInner::Var(_)
        | ExprInner::GasLeft(_)
        | ExprInner::Keccak(_)
        | ExprInner::Hash(_)
        | ExprInner::Not(_) => None,
        ExprInner::Ite(cond, then_expr, else_expr) => {
            if expr_const_value(then_expr).is_some_and(|value| !value.is_zero())
                && expr_const_value(else_expr).is_some_and(|value| value.is_zero())
            {
                bool_forces_expr_const_with_context(cond, target, context)
            } else {
                None
            }
        }
        ExprInner::Op(ExprOp::Or, left, right) => {
            if expr_const_value(left).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        ExprInner::Op(ExprOp::And, left, right) => {
            if expr_const_value(left).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(right, target, context);
            }
            if expr_const_value(right).is_some_and(|value| !value.is_zero()) {
                return expr_nonzero_forces_const(left, target, context);
            }
            None
        }
        ExprInner::Op(ExprOp::Shl | ExprOp::Shr, value, shift)
            if expr_const_value(shift).is_some_and(|shift| shift.is_zero()) =>
        {
            expr_nonzero_forces_const(value, target, context)
        }
        ExprInner::AddMod(_) | ExprInner::MulMod(_) => None,
        ExprInner::Op(_, _, _) => None,
    }
}

/// Returns whether `masked_expr_matches` holds.
fn masked_expr_matches(candidate: &ExprInner, target: &Expr) -> Option<U256> {
    match candidate {
        ExprInner::Op(ExprOp::And, left, right) if left == target => expr_const_value(right),
        ExprInner::Op(ExprOp::And, left, right) if right == target => expr_const_value(left),
        _ => None,
    }
}

pub(crate) fn context_forces_masked_expr(context: &[BoolExpr], target: &Expr, mask: U256) -> bool {
    context.iter().any(|condition| match condition.as_inner() {
        BoolExprRef::Eq(left, right) => {
            (left == target && masked_expr_matches(right.as_inner(), target) == Some(mask))
                || (right == target && masked_expr_matches(left.as_inner(), target) == Some(mask))
        }
        BoolExprRef::And(values) => context_forces_masked_expr(values, target, mask),
        _ => false,
    })
}

pub(crate) fn expr_const_value(expr: &Expr) -> Option<U256> {
    match expr.as_inner() {
        ExprInner::Const(value) => Some(*value),
        ExprInner::Var(_) | ExprInner::GasLeft(_) | ExprInner::Keccak(_) | ExprInner::Hash(_) => {
            None
        }
        ExprInner::Not(value) => Some(!expr_const_value(value)?),
        ExprInner::Op(op, left, right) => {
            Some(eval_expr_op(*op, expr_const_value(left)?, expr_const_value(right)?))
        }
        ExprInner::AddMod(expr) => Some(addmod_word(
            expr_const_value(expr.left())?,
            expr_const_value(expr.right())?,
            expr_const_value(expr.modulus())?,
        )),
        ExprInner::MulMod(expr) => Some(mulmod_word(
            expr_const_value(expr.left())?,
            expr_const_value(expr.right())?,
            expr_const_value(expr.modulus())?,
        )),
        ExprInner::Ite(cond, then_expr, else_expr) => {
            if bool_const_value(cond)? {
                expr_const_value(then_expr)
            } else {
                expr_const_value(else_expr)
            }
        }
    }
}

pub(crate) fn bool_const_value(expr: &BoolExpr) -> Option<bool> {
    match expr.as_inner() {
        BoolExprRef::Const(value) => Some(value),
        BoolExprRef::Not(value) => Some(!bool_const_value(value)?),
        BoolExprRef::And(values) => {
            let mut all_true = true;
            for value in values {
                all_true &= bool_const_value(value)?;
            }
            Some(all_true)
        }
        BoolExprRef::Eq(left, right) => Some(expr_const_value(left)? == expr_const_value(right)?),
        BoolExprRef::Cmp(op, left, right) => {
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

pub(crate) fn bool_contains_keccak(expr: &BoolExpr) -> bool {
    let mut contains = false;
    expr.visit_exprs(&mut |expr| contains |= matches!(expr.as_inner(), ExprInner::Keccak(_)));
    contains
}

/// Returns whether a boolean expression depends on the opaque `GAS` / `gasleft()` value.
pub(crate) fn bool_contains_gasleft(expr: &BoolExpr) -> bool {
    let mut contains = false;
    expr.visit_exprs(&mut |expr| contains |= matches!(expr.as_inner(), ExprInner::GasLeft(_)));
    contains
}

pub(crate) fn word_bytes(word: SymWord) -> Vec<SymWord> {
    if let Some(word) = word.as_const() {
        return word
            .to_be_bytes::<32>()
            .into_iter()
            .map(|byte| SymWord::constant(U256::from(byte)))
            .collect();
    }
    let expr = word.into_expr();
    (0..32).map(|idx| byte_expr(idx, &expr)).collect()
}

pub(crate) fn word_from_bytes(bytes: impl IntoIterator<Item = SymWord>) -> SymWord {
    let bytes = bytes.into_iter().collect::<Vec<_>>();
    if bytes.iter().all(|byte| byte.as_const().is_some()) {
        let mut word = [0u8; 32];
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            word[idx] = byte.as_const().expect("checked concrete byte").to::<u8>();
        }
        return SymWord::constant(U256::from_be_bytes(word));
    }

    if let Some(expr) = word_from_extracted_bytes(&bytes) {
        return SymWord::expr(expr);
    }

    let mut expr = Expr::constant(U256::ZERO);
    for (idx, byte) in bytes.into_iter().take(32).enumerate() {
        let shift = (31 - idx) * 8;
        let byte = low_byte(byte).into_expr();
        let byte = if shift == 0 {
            byte
        } else {
            Expr::op(ExprOp::Shl, byte, Expr::constant(U256::from(shift)))
        };
        expr = Expr::op(ExprOp::Or, expr, byte);
    }
    SymWord::expr(expr)
}

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

        let byte = byte.as_const()?;
        if expr_known_byte(&source, idx) != Some(byte.to::<u8>()) {
            return None;
        }
    }
    Some(source)
}

pub(crate) fn extracted_byte_source(byte: &SymWord, index: usize) -> Option<Expr> {
    let expr = byte.as_expr();
    let expr = strip_low_byte_mask(expr)?;
    if index == 31 {
        return Some(expr.clone());
    }
    let ExprInner::Op(ExprOp::Shr, source, shift) = expr.as_inner() else { return None };
    let shift = shift.as_const()?;
    (shift == U256::from((31 - index) * 8)).then(|| source.clone())
}

pub(crate) fn strip_low_byte_mask(expr: &Expr) -> Option<&Expr> {
    match expr.as_inner() {
        ExprInner::Op(ExprOp::And, left, right) if right.as_const() == Some(U256::from(0xff)) => {
            Some(strip_low_byte_mask(left).unwrap_or(left))
        }
        ExprInner::Op(ExprOp::And, left, right) if left.as_const() == Some(U256::from(0xff)) => {
            Some(strip_low_byte_mask(right).unwrap_or(right))
        }
        _ => Some(expr),
    }
}

pub(crate) fn low_byte(word: SymWord) -> SymWord {
    if let Some(word) = word.as_const() {
        return SymWord::constant(U256::from(word.to::<u8>()));
    }
    SymWord::expr(Expr::op(ExprOp::And, word.into_expr(), Expr::constant(U256::from(0xff))))
}

pub(crate) fn model_word(
    word: &SymWord,
    model: &(impl SymbolicModelLookup + ?Sized),
) -> Result<U256, SymbolicError> {
    if let Some(value) = word.as_const() {
        return Ok(value);
    }
    eval_expr(word.as_expr(), model)
}

pub(crate) fn model_bytes(
    bytes: &[SymWord],
    model: &(impl SymbolicModelLookup + ?Sized),
) -> Result<Vec<u8>, SymbolicError> {
    bytes.iter().map(|byte| Ok(model_word(byte, model)?.to::<u8>())).collect()
}

pub(crate) fn concrete_bytes(
    bytes: &[SymWord],
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
            _ if actual
                .as_const()
                .zip(expected.as_const())
                .is_some_and(|(actual, expected)| actual.to::<u8>() == expected.to::<u8>()) => {}
            _ if actual.as_const().is_some() && expected.as_const().is_some() => return Ok(None),
            _ => conditions.push(BoolExpr::eq_words(actual, expected)),
        }
    }
    Ok(Some(BoolExpr::and(conditions)))
}

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

pub(crate) fn eval_expr(
    expr: &Expr,
    model: &(impl SymbolicModelLookup + ?Sized),
) -> Result<U256, SymbolicError> {
    Ok(match expr.as_inner() {
        ExprInner::Const(value) => *value,
        ExprInner::Var(var) => model.value(*var).unwrap_or_default(),
        ExprInner::GasLeft(_) => {
            return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
        }
        ExprInner::Keccak(hash) => eval_keccak_expr(&hash.len, &hash.bytes, model)?,
        ExprInner::Hash(hash) => model.value(hash.name).unwrap_or_default(),
        ExprInner::Not(value) => !eval_expr(value, model)?,
        ExprInner::Op(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            eval_expr_op(*op, left, right)
        }
        ExprInner::AddMod(expr) => addmod_word(
            eval_expr(expr.left(), model)?,
            eval_expr(expr.right(), model)?,
            eval_expr(expr.modulus(), model)?,
        ),
        ExprInner::MulMod(expr) => mulmod_word(
            eval_expr(expr.left(), model)?,
            eval_expr(expr.right(), model)?,
            eval_expr(expr.modulus(), model)?,
        ),
        ExprInner::Ite(cond, then_expr, else_expr) => {
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
    model: &(impl SymbolicModelLookup + ?Sized),
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

pub(crate) fn eval_bool_expr(
    expr: &BoolExpr,
    model: &(impl SymbolicModelLookup + ?Sized),
) -> Result<bool, SymbolicError> {
    Ok(match expr.as_inner() {
        BoolExprRef::Const(value) => value,
        BoolExprRef::Not(value) => !eval_bool_expr(value, model)?,
        BoolExprRef::And(values) => {
            for value in values {
                if !eval_bool_expr(value, model)? {
                    return Ok(false);
                }
            }
            true
        }
        BoolExprRef::Eq(left, right) => eval_expr(left, model)? == eval_expr(right, model)?,
        BoolExprRef::Cmp(op, left, right) => {
            let left = eval_expr(left, model)?;
            let right = eval_expr(right, model)?;
            eval_bool_cmp(op, left, right)
        }
    })
}

/// Returns the concrete comparison result for a boolean expression.
pub(crate) fn eval_bool_cmp(op: BoolExprOp, left: U256, right: U256) -> bool {
    match op {
        BoolExprOp::Ult => left < right,
        BoolExprOp::Ugt => left > right,
        BoolExprOp::Ule => left <= right,
        BoolExprOp::Uge => left >= right,
        BoolExprOp::Slt => slt(left, right),
        BoolExprOp::Sgt => slt(right, left),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymWord(Expr);

impl SymWord {
    pub(crate) fn zero() -> Self {
        Self::constant(U256::ZERO)
    }

    /// Builds a concrete symbolic word.
    pub(crate) fn constant(value: U256) -> Self {
        Self(Expr::constant(value))
    }

    /// Returns the concrete value of this word.
    pub(crate) fn as_const(&self) -> Option<U256> {
        self.0.as_const()
    }

    /// Returns this word expression.
    pub(crate) const fn as_expr(&self) -> &Expr {
        &self.0
    }

    /// Clones this word expression.
    pub(crate) fn clone_expr(&self) -> Expr {
        self.0.clone()
    }

    /// Builds a symbolic word from an expression.
    pub(crate) const fn expr(expr: Expr) -> Self {
        Self(expr)
    }

    /// Returns whether this word depends on the opaque `GAS` / `gasleft()` value.
    pub(crate) fn contains_gasleft(&self) -> bool {
        expr_contains_gasleft(&self.0)
    }

    /// Returns whether this word is exactly the opaque `GAS` / `gasleft()` value.
    pub(crate) fn is_raw_gasleft(&self) -> bool {
        matches!(self.0.as_inner(), ExprInner::GasLeft(_))
    }

    pub(crate) fn into_expr(self) -> Expr {
        self.0
    }

    pub(crate) fn from_bool(value: BoolExpr) -> Self {
        match value.as_inner() {
            BoolExprRef::Const(value) => Self::constant(U256::from(value)),
            value => Self::expr(Expr::ite(
                value.to_owned(),
                Expr::constant(U256::from(1)),
                Expr::constant(U256::ZERO),
            )),
        }
    }

    pub(crate) fn truth(&self) -> Option<bool> {
        self.as_const().map(|value| !value.is_zero())
    }

    pub(crate) fn into_zero_bool(self) -> BoolExpr {
        if let Some(value) = self.as_const() {
            return BoolExpr::Const(value.is_zero());
        }
        match self.0.into_inner() {
            ExprInner::Ite(cond, then_expr, else_expr)
                if then_expr.as_const() == Some(U256::from(1))
                    && else_expr.as_const() == Some(U256::ZERO) =>
            {
                cond.not()
            }
            ExprInner::Ite(cond, then_expr, else_expr)
                if then_expr.as_const() == Some(U256::ZERO)
                    && else_expr.as_const() == Some(U256::from(1)) =>
            {
                cond
            }
            expr => BoolExpr::eq(Expr::from_inner(expr), Expr::constant(U256::ZERO)),
        }
    }

    pub(crate) fn nonzero_bool(self) -> BoolExpr {
        self.into_zero_bool().not()
    }

    pub(crate) fn into_concrete(self, reason: &'static str) -> Result<U256, SymbolicError> {
        self.as_const().ok_or(SymbolicError::Unsupported(reason))
    }

    pub(crate) fn into_usize(self, reason: &'static str) -> Result<usize, SymbolicError> {
        let value = self.into_concrete(reason)?;
        if value > U256::from(usize::MAX) {
            return Err(SymbolicError::Unsupported(reason));
        }
        Ok(value.to::<usize>())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Expr(Arc<ExprInner>);

impl fmt::Debug for Expr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_inner().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum ExprInner {
    Const(U256),
    Var(Symbol),
    GasLeft(usize),
    Keccak(KeccakExpr),
    Hash(HashExpr),
    Not(Expr),
    Op(ExprOp, Expr, Expr),
    AddMod(ModularExpr),
    MulMod(ModularExpr),
    Ite(BoolExpr, Expr, Expr),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct KeccakExpr {
    name: Symbol,
    len: Expr,
    bytes: Arc<[Expr]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct HashExpr {
    name: Symbol,
    algorithm: &'static str,
    bytes: Arc<[Expr]>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct ModularExpr {
    left: Expr,
    right: Expr,
    modulus: Expr,
}

impl KeccakExpr {
    pub(super) const fn len(&self) -> &Expr {
        &self.len
    }

    pub(super) fn bytes(&self) -> &[Expr] {
        &self.bytes
    }

    pub(super) fn into_parts(self) -> (Symbol, Expr, Arc<[Expr]>) {
        let Self { name, len, bytes } = self;
        (name, len, bytes)
    }
}

impl HashExpr {
    pub(super) const fn name(&self) -> Symbol {
        self.name
    }

    pub(super) fn into_parts(self) -> (Symbol, &'static str, Arc<[Expr]>) {
        let Self { name, algorithm, bytes } = self;
        (name, algorithm, bytes)
    }
}

impl ModularExpr {
    const fn new(left: Expr, right: Expr, modulus: Expr) -> Self {
        Self { left, right, modulus }
    }

    pub(super) const fn left(&self) -> &Expr {
        &self.left
    }

    pub(super) const fn right(&self) -> &Expr {
        &self.right
    }

    pub(super) const fn modulus(&self) -> &Expr {
        &self.modulus
    }

    pub(super) fn into_parts(self) -> (Expr, Expr, Expr) {
        let Self { left, right, modulus } = self;
        (left, right, modulus)
    }
}

impl Expr {
    fn from_inner(expr: ExprInner) -> Self {
        Self(Arc::new(expr))
    }

    /// Returns this expression's inner representation.
    pub(super) fn as_inner(&self) -> &ExprInner {
        self.0.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn var_name(&self) -> Option<&str> {
        match self.as_inner() {
            ExprInner::Var(name) => Some(name.as_str()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_keccak(&self) -> bool {
        matches!(self.as_inner(), ExprInner::Keccak(_))
    }

    #[cfg(test)]
    pub(crate) fn keccak_len_and_byte_count(&self) -> Option<(&Self, usize)> {
        match self.as_inner() {
            ExprInner::Keccak(hash) => Some((hash.len(), hash.bytes().len())),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn hash_algorithm(&self) -> Option<&'static str> {
        match self.as_inner() {
            ExprInner::Hash(hash) => Some(hash.algorithm),
            _ => None,
        }
    }

    fn into_inner(self) -> ExprInner {
        Arc::unwrap_or_clone(self.0)
    }

    /// Builds a concrete expression.
    pub(crate) fn constant(value: U256) -> Self {
        Self::from_inner(ExprInner::Const(value))
    }

    /// Returns this expression's concrete value.
    pub(crate) fn as_const(&self) -> Option<U256> {
        match self.as_inner() {
            ExprInner::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn var(name: impl AsRef<str>) -> Self {
        Self::var_symbol(Symbol::intern(name))
    }

    pub(crate) fn var_symbol(name: Symbol) -> Self {
        Self::from_inner(ExprInner::Var(name))
    }

    pub(crate) fn gas_left(id: usize) -> Self {
        Self::from_inner(ExprInner::GasLeft(id))
    }

    pub(crate) fn keccak(name: impl AsRef<str>, len: Self, bytes: Vec<Self>) -> Self {
        Self::keccak_symbol(Symbol::intern(name), len, bytes)
    }

    pub(crate) fn keccak_symbol(name: Symbol, len: Self, bytes: Vec<Self>) -> Self {
        Self::from_inner(ExprInner::Keccak(KeccakExpr { name, len, bytes: bytes.into() }))
    }

    pub(crate) fn hash(name: impl AsRef<str>, algorithm: &'static str, bytes: Vec<Self>) -> Self {
        Self::hash_symbol(Symbol::intern(name), algorithm, bytes)
    }

    pub(crate) fn hash_symbol(name: Symbol, algorithm: &'static str, bytes: Vec<Self>) -> Self {
        Self::from_inner(ExprInner::Hash(HashExpr { name, algorithm, bytes: bytes.into() }))
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
                    Self::from_inner(ExprInner::Ite(cond, then_expr, else_expr))
                }
            }
        }
    }

    /// Adds a concrete value to an expression.
    pub(crate) fn add_const(expr: Self, value: U256) -> Self {
        if value.is_zero() {
            return expr;
        }
        match expr.as_inner() {
            ExprInner::Const(expr) => Self::constant(expr.wrapping_add(value)),
            _ => Self::from_inner(ExprInner::Op(ExprOp::Add, expr, Self::constant(value))),
        }
    }

    /// Builds a bitwise-not expression.
    pub(crate) fn not(value: Self) -> Self {
        match value.into_inner() {
            ExprInner::Const(value) => Self::constant(!value),
            ExprInner::Not(value) => value,
            value => Self::from_inner(ExprInner::Not(Self::from_inner(value))),
        }
    }

    /// Visits this expression and all child expressions.
    pub(crate) fn visit(&self, visitor: &mut impl FnMut(&Self)) {
        visitor(self);
        match self.as_inner() {
            ExprInner::Const(_) | ExprInner::Var(_) | ExprInner::GasLeft(_) => {}
            ExprInner::Keccak(hash) => {
                hash.len.visit(visitor);
                for byte in hash.bytes.iter() {
                    byte.visit(visitor);
                }
            }
            ExprInner::Hash(hash) => {
                for byte in hash.bytes.iter() {
                    byte.visit(visitor);
                }
            }
            ExprInner::Not(value) => value.visit(visitor),
            ExprInner::Op(_, left, right) => {
                left.visit(visitor);
                right.visit(visitor);
            }
            ExprInner::AddMod(expr) | ExprInner::MulMod(expr) => {
                expr.left().visit(visitor);
                expr.right().visit(visitor);
                expr.modulus().visit(visitor);
            }
            ExprInner::Ite(cond, left, right) => {
                cond.visit_exprs(visitor);
                left.visit(visitor);
                right.visit(visitor);
            }
        }
    }

    pub(crate) fn op(op: ExprOp, left: Self, right: Self) -> Self {
        if let (Some(left), Some(right)) = (left.as_const(), right.as_const()) {
            return Self::constant(eval_expr_op(op, left, right));
        }

        match (op, left.as_inner(), right.as_inner()) {
            (ExprOp::Add, ExprInner::Const(value), _) if value.is_zero() => return right,
            (ExprOp::Add, _, ExprInner::Const(value)) if value.is_zero() => return left,
            (ExprOp::Sub, _, ExprInner::Const(value)) if value.is_zero() => return left,
            (ExprOp::Sub, _, _) if left == right => return Self::constant(U256::ZERO),
            (ExprOp::Mul, ExprInner::Const(value), _)
            | (ExprOp::Mul, _, ExprInner::Const(value))
                if value.is_zero() =>
            {
                return Self::constant(U256::ZERO);
            }
            (ExprOp::Mul, ExprInner::Const(value), _) if *value == U256::from(1) => return right,
            (ExprOp::Mul, _, ExprInner::Const(value)) if *value == U256::from(1) => return left,
            (
                ExprOp::UDiv | ExprOp::URem | ExprOp::SDiv | ExprOp::SRem,
                _,
                ExprInner::Const(value),
            ) if value.is_zero() => {
                return Self::constant(U256::ZERO);
            }
            (ExprOp::UDiv | ExprOp::SDiv, _, ExprInner::Const(value))
                if *value == U256::from(1) =>
            {
                return left;
            }
            (ExprOp::URem | ExprOp::SRem, _, ExprInner::Const(value))
                if *value == U256::from(1) =>
            {
                return Self::constant(U256::ZERO);
            }
            (ExprOp::And, ExprInner::Const(value), _)
            | (ExprOp::And, _, ExprInner::Const(value))
                if value.is_zero() =>
            {
                return Self::constant(U256::ZERO);
            }
            (ExprOp::And, ExprInner::Const(value), _) if *value == U256::MAX => return right,
            (ExprOp::And, _, ExprInner::Const(value)) if *value == U256::MAX => return left,
            (ExprOp::And, _, _) if left == right => return left,
            (ExprOp::And, ExprInner::Const(mask), _) => return Self::and_const(right, *mask),
            (ExprOp::And, _, ExprInner::Const(mask)) => return Self::and_const(left, *mask),
            (ExprOp::Or | ExprOp::Xor, ExprInner::Const(value), _)
            | (ExprOp::Or | ExprOp::Xor, _, ExprInner::Const(value))
                if value.is_zero() =>
            {
                return if matches!(left.as_inner(), ExprInner::Const(_)) { right } else { left };
            }
            (ExprOp::Shl | ExprOp::Shr | ExprOp::Sar, _, ExprInner::Const(value))
                if value.is_zero() =>
            {
                return left;
            }
            (ExprOp::Shl | ExprOp::Shr, ExprInner::Const(value), _) if value.is_zero() => {
                return Self::constant(U256::ZERO);
            }
            _ => {}
        }
        Self::from_inner(ExprInner::Op(op, left, right))
    }

    /// Builds an exact EVM `ADDMOD` expression.
    pub(crate) fn addmod(left: Self, right: Self, modulus: Self) -> Self {
        if modulus.as_const().is_some_and(|value| value.is_zero() || value == U256::from(1)) {
            return Self::constant(U256::ZERO);
        }
        if let (Some(left), Some(right), Some(modulus)) =
            (left.as_const(), right.as_const(), modulus.as_const())
        {
            return Self::constant(addmod_word(left, right, modulus));
        }
        Self::from_inner(ExprInner::AddMod(ModularExpr::new(left, right, modulus)))
    }

    /// Builds an exact EVM `MULMOD` expression.
    pub(crate) fn mulmod(left: Self, right: Self, modulus: Self) -> Self {
        if modulus.as_const().is_some_and(|value| value.is_zero() || value == U256::from(1)) {
            return Self::constant(U256::ZERO);
        }
        if let (Some(left), Some(right), Some(modulus)) =
            (left.as_const(), right.as_const(), modulus.as_const())
        {
            return Self::constant(mulmod_word(left, right, modulus));
        }
        Self::from_inner(ExprInner::MulMod(ModularExpr::new(left, right, modulus)))
    }

    fn and_const(expr: Self, mask: U256) -> Self {
        if mask.is_zero() {
            return Self::constant(U256::ZERO);
        }
        if mask == U256::MAX {
            return expr;
        }

        match expr.into_inner() {
            ExprInner::Op(ExprOp::And, left, right) => match (left, right) {
                (left, right) if left.as_const() == Some(mask) => Self::and_const(right, mask),
                (left, right) if right.as_const() == Some(mask) => Self::and_const(left, mask),
                (left, right) if left == right => Self::and_const(left, mask),
                (left, right) => Self::from_inner(ExprInner::Op(
                    ExprOp::And,
                    Self::from_inner(ExprInner::Op(ExprOp::And, left, right)),
                    Self::constant(mask),
                )),
            },
            expr => Self::from_inner(ExprInner::Op(
                ExprOp::And,
                Self::from_inner(expr),
                Self::constant(mask),
            )),
        }
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        self.visit(&mut |expr| match expr.as_inner() {
            ExprInner::Var(var) => {
                vars.insert(*var);
            }
            ExprInner::Keccak(hash) => {
                vars.insert(hash.name);
            }
            ExprInner::Hash(hash) => {
                vars.insert(hash.name);
            }
            ExprInner::Const(_)
            | ExprInner::GasLeft(_)
            | ExprInner::Not(_)
            | ExprInner::Op(_, _, _)
            | ExprInner::AddMod(_)
            | ExprInner::MulMod(_)
            | ExprInner::Ite(_, _, _) => {}
        });
    }

    #[cfg(test)]
    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    fn write_smt(&self, out: &mut String) {
        match self.as_inner() {
            ExprInner::Const(value) => {
                let _ = write!(out, "(_ bv{value} 256)");
            }
            ExprInner::Var(var) => out.push_str(var.as_str()),
            ExprInner::GasLeft(id) => {
                let _ = write!(out, "gasleft_{id}");
            }
            ExprInner::Keccak(hash) => out.push_str(hash.name.as_str()),
            ExprInner::Hash(hash) => out.push_str(hash.name.as_str()),
            ExprInner::Not(value) => {
                out.push_str("(bvnot ");
                value.write_smt(out);
                out.push(')');
            }
            ExprInner::Op(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            ExprInner::AddMod(expr) => {
                write_smt_wide_modular_arithmetic(
                    out,
                    "bvadd",
                    expr.left(),
                    expr.right(),
                    expr.modulus(),
                );
            }
            ExprInner::MulMod(expr) => {
                write_smt_wide_modular_arithmetic(
                    out,
                    "bvmul",
                    expr.left(),
                    expr.right(),
                    expr.modulus(),
                );
            }
            ExprInner::Ite(cond, left, right) => {
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

/// Encodes EVM `ADDMOD`/`MULMOD` by widening operands before modular reduction.
fn write_smt_wide_modular_arithmetic(
    out: &mut String,
    op: &'static str,
    left: &Expr,
    right: &Expr,
    modulus: &Expr,
) {
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

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoolExpr {
    Const(bool),
    Inner(Arc<BoolExprInner>),
}

impl fmt::Debug for BoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Const(value) => f.debug_tuple("Const").field(value).finish(),
            Self::Inner(inner) => inner.fmt(f),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum BoolExprInner {
    Not(BoolExpr),
    And(Arc<[BoolExpr]>),
    Eq(Expr, Expr),
    Cmp(BoolExprOp, Expr, Expr),
}

#[derive(Clone, Copy)]
pub(crate) enum BoolExprRef<'a> {
    Const(bool),
    Not(&'a BoolExpr),
    And(&'a [BoolExpr]),
    Eq(&'a Expr, &'a Expr),
    Cmp(BoolExprOp, &'a Expr, &'a Expr),
}

impl BoolExprRef<'_> {
    fn to_owned(self) -> BoolExpr {
        match self {
            Self::Const(value) => BoolExpr::Const(value),
            Self::Not(value) => BoolExpr::from_inner(BoolExprInner::Not(value.clone())),
            Self::And(values) => {
                BoolExpr::from_inner(BoolExprInner::And(values.iter().cloned().collect()))
            }
            Self::Eq(left, right) => {
                BoolExpr::from_inner(BoolExprInner::Eq(left.clone(), right.clone()))
            }
            Self::Cmp(op, left, right) => {
                BoolExpr::from_inner(BoolExprInner::Cmp(op, left.clone(), right.clone()))
            }
        }
    }
}

pub(crate) enum BoolExprOwned {
    Const(bool),
    Not(BoolExpr),
    And(Arc<[BoolExpr]>),
    Eq(Expr, Expr),
    Cmp(BoolExprOp, Expr, Expr),
}

impl BoolExpr {
    fn from_inner(expr: BoolExprInner) -> Self {
        Self::Inner(Arc::new(expr))
    }

    pub(crate) fn as_inner(&self) -> BoolExprRef<'_> {
        match self {
            Self::Const(value) => BoolExprRef::Const(*value),
            Self::Inner(inner) => match inner.as_ref() {
                BoolExprInner::Not(value) => BoolExprRef::Not(value),
                BoolExprInner::And(values) => BoolExprRef::And(values),
                BoolExprInner::Eq(left, right) => BoolExprRef::Eq(left, right),
                BoolExprInner::Cmp(op, left, right) => BoolExprRef::Cmp(*op, left, right),
            },
        }
    }

    pub(crate) fn into_inner(self) -> BoolExprOwned {
        match self {
            Self::Const(value) => BoolExprOwned::Const(value),
            Self::Inner(inner) => match Arc::unwrap_or_clone(inner) {
                BoolExprInner::Not(value) => BoolExprOwned::Not(value),
                BoolExprInner::And(values) => BoolExprOwned::And(values),
                BoolExprInner::Eq(left, right) => BoolExprOwned::Eq(left, right),
                BoolExprInner::Cmp(op, left, right) => BoolExprOwned::Cmp(op, left, right),
            },
        }
    }

    /// Visits this boolean expression and all child boolean expressions.
    pub(crate) fn visit(&self, visitor: &mut impl FnMut(&Self)) {
        visitor(self);
        match self.as_inner() {
            BoolExprRef::Const(_) | BoolExprRef::Eq(_, _) | BoolExprRef::Cmp(_, _, _) => {}
            BoolExprRef::Not(value) => value.visit(visitor),
            BoolExprRef::And(values) => {
                for value in values {
                    value.visit(visitor);
                }
            }
        }
    }

    /// Visits all word expressions contained in this boolean expression.
    pub(crate) fn visit_exprs(&self, visitor: &mut impl FnMut(&Expr)) {
        match self.as_inner() {
            BoolExprRef::Const(_) => {}
            BoolExprRef::Not(value) => value.visit_exprs(visitor),
            BoolExprRef::And(values) => {
                for value in values {
                    value.visit_exprs(visitor);
                }
            }
            BoolExprRef::Eq(left, right) | BoolExprRef::Cmp(_, left, right) => {
                left.visit(visitor);
                right.visit(visitor);
            }
        }
    }

    pub(crate) fn eq(left: Expr, right: Expr) -> Self {
        if left == right {
            return Self::Const(true);
        }
        match (left.as_inner(), right.as_inner()) {
            (ExprInner::Const(left), ExprInner::Const(right)) => Self::Const(left == right),
            (_, ExprInner::Const(right_value)) => {
                if let Some(left_value) = expr_known_word(&left) {
                    return Self::Const(left_value == *right_value);
                }
                Self::from_inner(BoolExprInner::Eq(left, right))
            }
            (ExprInner::Const(left_value), _) => {
                if let Some(right_value) = expr_known_word(&right) {
                    return Self::Const(*left_value == right_value);
                }
                Self::from_inner(BoolExprInner::Eq(left, right))
            }
            (ExprInner::Keccak(left), ExprInner::Keccak(right))
                if left.bytes.len() == right.bytes.len() =>
            {
                let mut conditions = vec![Self::eq(left.len.clone(), right.len.clone())];
                conditions.extend(
                    left.bytes
                        .iter()
                        .cloned()
                        .zip(right.bytes.iter().cloned())
                        .map(|(left, right)| Self::eq(left, right)),
                );
                Self::and(conditions)
            }
            (ExprInner::Hash(left), ExprInner::Hash(right))
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
            _ => Self::from_inner(BoolExprInner::Eq(left, right)),
        }
    }

    /// Builds equality between a symbolic word and a concrete value.
    pub(crate) fn eq_word_const(word: &SymWord, value: U256) -> Self {
        if let Some(word) = word.as_const() {
            Self::Const(word == value)
        } else {
            Self::eq(word.as_expr().clone(), Expr::constant(value))
        }
    }

    /// Builds equality between a symbolic word and an owned expression.
    pub(crate) fn eq_word_expr(word: &SymWord, expr: Expr) -> Self {
        Self::eq(word.as_expr().clone(), expr)
    }

    /// Builds equality between borrowed symbolic words.
    pub(crate) fn eq_words(left: &SymWord, right: &SymWord) -> Self {
        Self::eq(left.as_expr().clone(), right.as_expr().clone())
    }

    pub(crate) fn and(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.as_inner() {
                BoolExprRef::Const(true) => {}
                BoolExprRef::Const(false) => return Self::Const(false),
                BoolExprRef::And(values) => out.extend(values.iter().cloned()),
                _ => out.push(value),
            }
        }
        if out.is_empty() {
            Self::Const(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::from_inner(BoolExprInner::And(out.into()))
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_and(values: Vec<Self>) -> Self {
        Self::from_inner(BoolExprInner::And(values.into()))
    }

    pub(crate) fn or(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.as_inner() {
                BoolExprRef::Const(false) => {}
                BoolExprRef::Const(true) => return Self::Const(true),
                _ => out.push(value),
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

    pub(crate) fn cmp(op: BoolExprOp, left: Expr, right: Expr) -> Self {
        if left == right {
            return Self::Const(matches!(op, BoolExprOp::Ule | BoolExprOp::Uge));
        }
        if let (Some(left), Some(right)) = (left.as_const(), right.as_const()) {
            return Self::Const(eval_bool_cmp(op, left, right));
        }
        match (op, left.as_inner(), right.as_inner()) {
            (BoolExprOp::Ugt, ExprInner::Const(value), _) if value.is_zero() => {
                return Self::Const(false);
            }
            (BoolExprOp::Ule, ExprInner::Const(value), _) if value.is_zero() => {
                return Self::Const(true);
            }
            (BoolExprOp::Ult, _, ExprInner::Const(value)) if value.is_zero() => {
                return Self::Const(false);
            }
            (BoolExprOp::Uge, _, ExprInner::Const(value)) if value.is_zero() => {
                return Self::Const(true);
            }
            (BoolExprOp::Ult, ExprInner::Const(value), _) if *value == U256::MAX => {
                return Self::Const(false);
            }
            (BoolExprOp::Uge, ExprInner::Const(value), _) if *value == U256::MAX => {
                return Self::Const(true);
            }
            (BoolExprOp::Ugt, _, ExprInner::Const(value)) if *value == U256::MAX => {
                return Self::Const(false);
            }
            (BoolExprOp::Ule, _, ExprInner::Const(value)) if *value == U256::MAX => {
                return Self::Const(true);
            }
            _ => {}
        }
        Self::from_inner(BoolExprInner::Cmp(op, left, right))
    }

    /// Builds comparison between a symbolic word and a concrete value.
    pub(crate) fn cmp_word_const(op: BoolExprOp, word: &SymWord, value: U256) -> Self {
        if let Some(word) = word.as_const() {
            Self::Const(eval_bool_cmp(op, word, value))
        } else {
            Self::cmp(op, word.as_expr().clone(), Expr::constant(value))
        }
    }

    /// Builds comparison between a symbolic word and an owned expression.
    pub(crate) fn cmp_word_expr(op: BoolExprOp, word: &SymWord, expr: Expr) -> Self {
        Self::cmp(op, word.as_expr().clone(), expr)
    }

    pub(crate) fn not(self) -> Self {
        match self.as_inner() {
            BoolExprRef::Const(value) => Self::Const(!value),
            BoolExprRef::Not(value) => value.clone(),
            _ => Self::from_inner(BoolExprInner::Not(self)),
        }
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        self.visit(&mut |expr| match expr.as_inner() {
            BoolExprRef::Eq(left, right) | BoolExprRef::Cmp(_, left, right) => {
                left.collect_vars(vars);
                right.collect_vars(vars);
            }
            BoolExprRef::Const(_) | BoolExprRef::Not(_) | BoolExprRef::And(_) => {}
        });
    }

    pub(crate) fn smt(&self) -> String {
        let mut smt = String::new();
        self.write_smt(&mut smt);
        smt
    }

    fn write_smt(&self, out: &mut String) {
        match self.as_inner() {
            BoolExprRef::Const(value) => out.push_str(if value { "true" } else { "false" }),
            BoolExprRef::Not(value) => {
                out.push_str("(not ");
                value.write_smt(out);
                out.push(')');
            }
            BoolExprRef::And(values) => {
                out.push_str("(and");
                for value in values {
                    out.push(' ');
                    value.write_smt(out);
                }
                out.push(')');
            }
            BoolExprRef::Eq(left, right) => {
                out.push_str("(= ");
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            BoolExprRef::Cmp(op, left, right) => {
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
pub(crate) enum BoolExprOp {
    Ult,
    Ugt,
    Ule,
    Uge,
    Slt,
    Sgt,
}

impl BoolExprOp {
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

pub(crate) fn u256_to_usize(value: U256) -> Option<usize> {
    if value > U256::from(usize::MAX) { None } else { Some(value.to::<usize>()) }
}

pub(crate) fn bool_upper_bound_usize(condition: &BoolExpr, expr: &Expr) -> Option<usize> {
    match condition.as_inner() {
        BoolExprRef::Const(_) | BoolExprRef::Not(_) => None,
        BoolExprRef::And(values) => {
            let mut bound: Option<usize> = None;
            for value in values {
                if let Some(candidate) = bool_upper_bound_usize(value, expr) {
                    bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                }
            }
            bound
        }
        BoolExprRef::Eq(left, right) => match (left == expr, right == expr) {
            (true, _) => expr_const_value(right).and_then(u256_to_usize),
            (_, true) => expr_const_value(left).and_then(u256_to_usize),
            _ => None,
        },
        BoolExprRef::Cmp(op, left, right) => {
            if left == expr {
                match op {
                    BoolExprOp::Ult => expr_const_value(right)
                        .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                        .and_then(u256_to_usize),
                    BoolExprOp::Ule => expr_const_value(right).and_then(u256_to_usize),
                    _ => None,
                }
            } else if right == expr {
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
