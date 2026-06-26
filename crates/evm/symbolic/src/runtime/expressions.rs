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
    pub(crate) fn intern(name: &str) -> Self {
        SYMBOL_INTERNER.intern(name)
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

pub(crate) fn keccak_word(bytes: Vec<SymExpr>) -> SymExpr {
    let len = bytes.len();
    keccak_word_with_len(bytes, SymExpr::constant(U256::from(len)))
}

pub(crate) fn keccak_word_with_len(bytes: Vec<SymExpr>, len: SymExpr) -> SymExpr {
    if bytes.iter().all(|byte| byte.as_const().is_some())
        && let Some(len) = len.as_const()
        && let Ok(len) = usize::try_from(len)
        && len <= bytes.len()
    {
        let bytes = bytes
            .into_iter()
            .take(len)
            .map(|byte| byte.as_const().expect("checked concrete byte").to::<u8>())
            .collect::<Vec<_>>();
        return SymExpr::constant(U256::from_be_bytes(keccak256(bytes).0));
    }

    let exprs = bytes;
    let name = stable_symbol("keccak", format!("{len:?}:{exprs:?}").as_bytes());
    SymExpr::keccak(&name, len, exprs)
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
    SymExpr::hash(&name, algorithm, identity)
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
            let initcode_bytes = initcode.bytes().to_vec();
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
        if nonce > U256::from(u64::MAX) {
            return Err(SymbolicError::Unsupported("symbolic vm.computeCreateAddress nonce"));
        }
        return Ok(SymExpr::constant(address_word(deployer.create(nonce.to()))));
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
    let word = SymExpr::var(&name);
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
    let word = SymExpr::var(&name);
    state.constraints.push(SymBoolExpr::cmp_word_const(
        SymBoolExprOp::Ult,
        &word,
        U256::from(1) << 160,
    ));
    word
}

pub(crate) fn read_storage_writes(
    writes: &[StorageWrite],
    address: Address,
    key: SymExpr,
    base: SymExpr,
) -> SymExpr {
    let mut value = base;
    for write in writes.iter().filter(|write| write.address == address) {
        value = storage_select(key.clone(), write.key.clone(), write.value.clone(), value);
    }
    value
}

pub(crate) fn storage_select(
    read_key: SymExpr,
    write_key: SymExpr,
    write_value: SymExpr,
    base: SymExpr,
) -> SymExpr {
    if write_value == base {
        return base;
    }
    let condition = storage_key_eq(read_key, write_key);
    match condition.as_const() {
        Some(true) => write_value,
        Some(false) => base,
        None => SymExpr::ite(condition, write_value, base),
    }
}

pub(crate) fn storage_key_eq(read_key: SymExpr, write_key: SymExpr) -> SymBoolExpr {
    if let (Some(read_root), Some(write_root)) =
        (storage_mapping_root_slot(&read_key), storage_mapping_root_slot(&write_key))
        && read_root != write_root
    {
        return SymBoolExpr::constant(false);
    }
    match (storage_layout_key(&read_key), storage_layout_key(&write_key)) {
        (Some((read_base, read_offset)), Some((write_base, write_offset))) => {
            SymBoolExpr::and(vec![
                SymBoolExpr::eq(read_base, write_base),
                SymBoolExpr::eq(read_offset, write_offset),
            ])
        }
        (Some(_), None) if write_key.as_const().is_some() => SymBoolExpr::constant(false),
        (None, Some(_)) if read_key.as_const().is_some() => SymBoolExpr::constant(false),
        _ => SymBoolExpr::eq(read_key, write_key),
    }
}

/// Returns the root Solidity storage slot for a mapping-style keccak key.
pub(crate) fn storage_mapping_root_slot(key: &SymExpr) -> Option<U256> {
    let SymExprInner::Keccak { len, bytes, .. } = key.as_inner() else { return None };
    if len.as_const() != Some(U256::from(64)) || bytes.len() < 64 {
        return None;
    }

    let slot = word_from_bytes(bytes[32..64].iter().cloned());
    match slot.as_inner() {
        SymExprInner::Const(slot) => Some(*slot),
        SymExprInner::Keccak { .. } => storage_mapping_root_slot(&slot),
        _ => None,
    }
}

pub(crate) fn storage_layout_key(key: &SymExpr) -> Option<(SymExpr, SymExpr)> {
    match key.as_inner() {
        SymExprInner::Keccak { .. } => Some((key.clone(), SymExpr::constant(U256::ZERO))),
        SymExprInner::Op(SymExprOp::Add, left, right) => {
            if let Some((base, offset)) = storage_layout_key(left)
                && !right.contains_keccak()
            {
                return Some((base, SymExpr::op(SymExprOp::Add, offset, right.clone())));
            }
            if let Some((base, offset)) = storage_layout_key(right)
                && !left.contains_keccak()
            {
                return Some((base, SymExpr::op(SymExprOp::Add, offset, left.clone())));
            }
            None
        }
        _ => None,
    }
}

/// Computes the exact EVM `ADDMOD` semantics without truncating the intermediate sum.
pub(crate) fn addmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    left.add_mod(right, modulus)
}

/// Computes the exact EVM `MULMOD` semantics without truncating the intermediate product.
pub(crate) fn mulmod_word(left: U256, right: U256, modulus: U256) -> U256 {
    left.mul_mod(right, modulus)
}

fn masked_expr_matches(candidate: &SymExprInner, target: &SymExpr) -> Option<U256> {
    match candidate {
        SymExprInner::Op(SymExprOp::And, left, right) if left == target => right.eval_const(),
        SymExprInner::Op(SymExprOp::And, left, right) if right == target => left.eval_const(),
        _ => None,
    }
}

fn context_forces_masked_expr(context: &[SymBoolExpr], target: &SymExpr, mask: U256) -> bool {
    context.iter().any(|condition| match condition.as_inner() {
        SymBoolExprInner::Eq(left, right) => {
            (left == target && masked_expr_matches(right.as_inner(), target) == Some(mask))
                || (right == target && masked_expr_matches(left.as_inner(), target) == Some(mask))
        }
        SymBoolExprInner::And(values) => context_forces_masked_expr(values, target, mask),
        _ => false,
    })
}

pub(crate) fn word_bytes(word: SymExpr) -> Vec<SymExpr> {
    if let Some(word) = word.as_const() {
        return word
            .to_be_bytes::<32>()
            .into_iter()
            .map(|byte| SymExpr::constant(U256::from(byte)))
            .collect();
    }
    let expr = word;
    (0..32).map(|idx| byte_expr(idx, &expr)).collect()
}

pub(crate) fn word_from_bytes(bytes: impl IntoIterator<Item = SymExpr>) -> SymExpr {
    let bytes = bytes.into_iter().collect::<Vec<_>>();
    if bytes.iter().all(|byte| byte.as_const().is_some()) {
        let mut word = [0u8; 32];
        for (idx, byte) in bytes.into_iter().take(32).enumerate() {
            word[idx] = byte.as_const().expect("checked concrete byte").to::<u8>();
        }
        return SymExpr::constant(U256::from_be_bytes(word));
    }

    if let Some(expr) = word_from_extracted_bytes(&bytes) {
        return expr;
    }

    let mut expr = SymExpr::constant(U256::ZERO);
    for (idx, byte) in bytes.into_iter().take(32).enumerate() {
        let shift = (31 - idx) * 8;
        let byte = low_byte(byte);
        let byte = if shift == 0 {
            byte
        } else {
            SymExpr::op(SymExprOp::Shl, byte, SymExpr::constant(U256::from(shift)))
        };
        expr = SymExpr::op(SymExprOp::Or, expr, byte);
    }
    expr
}

pub(crate) fn word_from_extracted_bytes(bytes: &[SymExpr]) -> Option<SymExpr> {
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
        if source.known_byte(idx) != Some(byte.to::<u8>()) {
            return None;
        }
    }
    Some(source)
}

pub(crate) fn extracted_byte_source(byte: &SymExpr, index: usize) -> Option<SymExpr> {
    let expr = byte;
    let expr = strip_low_byte_mask(expr)?;
    if index == 31 {
        return Some(expr.clone());
    }
    let SymExprInner::Op(SymExprOp::Shr, source, shift) = expr.as_inner() else { return None };
    let shift = shift.as_const()?;
    (shift == U256::from((31 - index) * 8)).then(|| source.clone())
}

pub(crate) fn strip_low_byte_mask(expr: &SymExpr) -> Option<&SymExpr> {
    match expr.as_inner() {
        SymExprInner::Op(SymExprOp::And, left, right)
            if right.as_const() == Some(U256::from(0xff)) =>
        {
            Some(strip_low_byte_mask(left).unwrap_or(left))
        }
        SymExprInner::Op(SymExprOp::And, left, right)
            if left.as_const() == Some(U256::from(0xff)) =>
        {
            Some(strip_low_byte_mask(right).unwrap_or(right))
        }
        _ => Some(expr),
    }
}

pub(crate) fn low_byte(word: SymExpr) -> SymExpr {
    if let Some(word) = word.as_const() {
        return SymExpr::constant(U256::from(word.to::<u8>()));
    }
    SymExpr::op(SymExprOp::And, word, SymExpr::constant(U256::from(0xff)))
}

pub(crate) fn concrete_bytes(
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

pub(crate) fn calldata_prefix_condition(
    calldata: &[SymExpr],
    prefix: &[SymExpr],
    _reason: &'static str,
) -> Result<Option<SymBoolExpr>, SymbolicError> {
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
            _ => conditions.push(SymBoolExpr::eq_words(actual, expected)),
        }
    }
    Ok(Some(SymBoolExpr::and(conditions)))
}

pub(crate) fn function_mock_match_condition(
    mock: &FunctionMock,
    callee: Address,
    calldata: &[SymExpr],
    reason: &'static str,
) -> Result<Option<SymBoolExpr>, SymbolicError> {
    let Some(data_condition) = calldata_prefix_condition(calldata, &mock.data, reason)? else {
        return Ok(None);
    };
    Ok(Some(SymBoolExpr::and(vec![address_match_condition(&mock.callee, callee), data_condition])))
}

pub(crate) trait SymExprSliceExt {
    fn eval<M: SymbolicModelLookup + ?Sized>(&self, model: &M) -> Result<Vec<u8>, SymbolicError>;
}

impl SymExprSliceExt for [SymExpr] {
    fn eval<M: SymbolicModelLookup + ?Sized>(&self, model: &M) -> Result<Vec<u8>, SymbolicError> {
        self.iter().map(|byte| Ok(byte.eval(model)?.to::<u8>())).collect()
    }
}

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymExpr(Arc<SymExprInner>);

impl fmt::Debug for SymExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_inner().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum SymExprInner {
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

static EXPR_ZERO: LazyLock<Arc<SymExprInner>> =
    LazyLock::new(|| Arc::new(SymExprInner::Const(U256::ZERO)));
static EXPR_ONE: LazyLock<Arc<SymExprInner>> =
    LazyLock::new(|| Arc::new(SymExprInner::Const(U256::from(1))));
static EXPR_MAX: LazyLock<Arc<SymExprInner>> =
    LazyLock::new(|| Arc::new(SymExprInner::Const(U256::MAX)));

impl SymExpr {
    fn from_inner(expr: SymExprInner) -> Self {
        match expr {
            SymExprInner::Const(value) => Self::constant(value),
            expr => Self(Arc::new(expr)),
        }
    }

    pub(crate) fn zero() -> Self {
        Self::constant(U256::ZERO)
    }

    pub(super) fn as_inner(&self) -> &SymExprInner {
        self.0.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn var_name(&self) -> Option<&str> {
        match self.as_inner() {
            SymExprInner::Var(name) => Some(name.as_str()),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn is_keccak(&self) -> bool {
        matches!(self.as_inner(), SymExprInner::Keccak { .. })
    }

    #[cfg(test)]
    pub(crate) fn keccak_len_and_byte_count(&self) -> Option<(&Self, usize)> {
        match self.as_inner() {
            SymExprInner::Keccak { len, bytes, .. } => Some((len, bytes.len())),
            _ => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn hash_algorithm(&self) -> Option<&'static str> {
        match self.as_inner() {
            SymExprInner::Hash { algorithm, .. } => Some(algorithm),
            _ => None,
        }
    }

    pub(super) fn into_inner(self) -> SymExprInner {
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
            Self(Arc::new(SymExprInner::Const(value)))
        }
    }

    pub(crate) fn as_const(&self) -> Option<U256> {
        match self.as_inner() {
            SymExprInner::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn eval_const(&self) -> Option<U256> {
        match self.as_inner() {
            SymExprInner::Const(value) => Some(*value),
            SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. } => None,
            SymExprInner::Not(value) => Some(!value.eval_const()?),
            SymExprInner::Op(op, left, right) => {
                Some(op.eval(left.eval_const()?, right.eval_const()?))
            }
            SymExprInner::AddMod { left, right, modulus } => {
                Some(addmod_word(left.eval_const()?, right.eval_const()?, modulus.eval_const()?))
            }
            SymExprInner::MulMod { left, right, modulus } => {
                Some(mulmod_word(left.eval_const()?, right.eval_const()?, modulus.eval_const()?))
            }
            SymExprInner::Ite(cond, then_expr, else_expr) => {
                if cond.eval_const()? {
                    then_expr.eval_const()
                } else {
                    else_expr.eval_const()
                }
            }
        }
    }

    pub(crate) fn eval<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<U256, SymbolicError> {
        Ok(match self.as_inner() {
            SymExprInner::Const(value) => *value,
            SymExprInner::Var(var) => model.value(*var).unwrap_or_default(),
            SymExprInner::GasLeft(_) => {
                return Err(SymbolicError::Unsupported("GAS/gasleft() not modeled"));
            }
            SymExprInner::Keccak { len, bytes, .. } => {
                let len = len.eval(model)?;
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
                    input.push((byte.eval(model)? & U256::from(0xff)).to::<u8>());
                }

                U256::from_be_bytes(keccak256(input).0)
            }
            SymExprInner::Hash { name, .. } => model.value(*name).unwrap_or_default(),
            SymExprInner::Not(value) => !value.eval(model)?,
            SymExprInner::Op(op, left, right) => op.eval(left.eval(model)?, right.eval(model)?),
            SymExprInner::AddMod { left, right, modulus } => {
                addmod_word(left.eval(model)?, right.eval(model)?, modulus.eval(model)?)
            }
            SymExprInner::MulMod { left, right, modulus } => {
                mulmod_word(left.eval(model)?, right.eval(model)?, modulus.eval(model)?)
            }
            SymExprInner::Ite(cond, then_expr, else_expr) => {
                if cond.eval(model)? {
                    then_expr.eval(model)?
                } else {
                    else_expr.eval(model)?
                }
            }
        })
    }

    pub(crate) fn from_bool(value: SymBoolExpr) -> Self {
        match value.into_inner() {
            SymBoolExprInner::Const(value) => Self::constant(U256::from(value)),
            value => Self::ite(
                SymBoolExpr::from_inner(value),
                Self::constant(U256::from(1)),
                Self::constant(U256::ZERO),
            ),
        }
    }

    pub(crate) fn truth(&self) -> Option<bool> {
        self.as_const().map(|value| !value.is_zero())
    }

    pub(crate) fn into_zero_bool(self) -> SymBoolExpr {
        match self.into_inner() {
            SymExprInner::Const(value) => SymBoolExpr::constant(value.is_zero()),
            SymExprInner::Ite(cond, then_expr, else_expr)
                if then_expr.as_const() == Some(U256::from(1))
                    && else_expr.as_const() == Some(U256::ZERO) =>
            {
                cond.not()
            }
            SymExprInner::Ite(cond, then_expr, else_expr)
                if then_expr.as_const() == Some(U256::ZERO)
                    && else_expr.as_const() == Some(U256::from(1)) =>
            {
                cond
            }
            expr => SymBoolExpr::eq(Self::from_inner(expr), Self::constant(U256::ZERO)),
        }
    }

    pub(crate) fn nonzero_bool(self) -> SymBoolExpr {
        self.into_zero_bool().not()
    }

    pub(crate) fn into_concrete(self, reason: &'static str) -> Result<U256, SymbolicError> {
        match self.into_inner() {
            SymExprInner::Const(value) => Ok(value),
            _ => Err(SymbolicError::Unsupported(reason)),
        }
    }

    pub(crate) fn into_usize(self, reason: &'static str) -> Result<usize, SymbolicError> {
        let value = self.into_concrete(reason)?;
        usize::try_from(value).map_err(|_| SymbolicError::Unsupported(reason))
    }

    pub(crate) fn contains_keccak(&self) -> bool {
        self.visit(&mut |expr| {
            if matches!(expr.as_inner(), SymExprInner::Keccak { .. }) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        self.visit(&mut |expr| {
            if matches!(expr.as_inner(), SymExprInner::GasLeft(_)) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    pub(crate) fn known_byte(&self, index: usize) -> Option<u8> {
        debug_assert!(index < 32);
        match self.as_inner() {
            SymExprInner::Const(value) => Some(value.to_be_bytes::<32>()[index]),
            SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. } => None,
            SymExprInner::Not(value) => value.known_byte(index).map(|byte| !byte),
            SymExprInner::Ite(_, then_expr, else_expr) => {
                let then_byte = then_expr.known_byte(index)?;
                let else_byte = else_expr.known_byte(index)?;
                (then_byte == else_byte).then_some(then_byte)
            }
            SymExprInner::Op(op, left, right) => match op {
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
            SymExprInner::AddMod { .. } | SymExprInner::MulMod { .. } => None,
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

    pub(crate) fn byte_term(&self, index: usize) -> Option<Self> {
        debug_assert!(index < 32);

        match self.as_inner() {
            SymExprInner::Const(value) => {
                Some(Self::constant(U256::from(value.to_be_bytes::<32>()[index])))
            }
            SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. } => Some(self.extracted_byte(index)),
            SymExprInner::Not(value) => Some(Self::not(value.byte_term(index)?)),
            SymExprInner::Ite(cond, then_expr, else_expr) => Some(Self::ite(
                cond.clone(),
                then_expr.byte_term(index)?,
                else_expr.byte_term(index)?,
            )),
            SymExprInner::Op(op, left, right) => match op {
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
                    let shift = right.eval_const()?;
                    if shift >= U256::from(256) {
                        return Some(Self::constant(U256::ZERO));
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let source_index = index + shift / 8;
                    if source_index >= 32 {
                        Some(Self::constant(U256::ZERO))
                    } else {
                        left.byte_term(source_index)
                    }
                }
                SymExprOp::Shr => {
                    let shift = right.eval_const()?;
                    if shift >= U256::from(256) {
                        return Some(Self::constant(U256::ZERO));
                    }
                    let shift = usize::try_from(shift).expect("checked byte shift");
                    if shift % 8 != 0 {
                        return None;
                    }
                    let byte_shift = shift / 8;
                    if index < byte_shift {
                        Some(Self::constant(U256::ZERO))
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
            SymExprInner::AddMod { .. } | SymExprInner::MulMod { .. } => None,
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
        let mask = masked_expr_matches(self.as_inner(), expr)?;
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
        match self.as_inner() {
            SymExprInner::Const(_)
            | SymExprInner::Var(_)
            | SymExprInner::GasLeft(_)
            | SymExprInner::Keccak { .. }
            | SymExprInner::Hash { .. }
            | SymExprInner::Not(_) => None,
            SymExprInner::Ite(cond, then_expr, else_expr) => {
                if then_expr.eval_const().is_some_and(|value| !value.is_zero())
                    && else_expr.eval_const().is_some_and(|value| value.is_zero())
                {
                    cond.forces_expr_const_with_context(target, context)
                } else {
                    None
                }
            }
            SymExprInner::Op(SymExprOp::Or, left, right) => {
                if left.eval_const().is_some_and(|value| value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval_const().is_some_and(|value| value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprInner::Op(SymExprOp::And, left, right) => {
                if left.eval_const().is_some_and(|value| !value.is_zero()) {
                    return right.nonzero_forces_const(target, context);
                }
                if right.eval_const().is_some_and(|value| !value.is_zero()) {
                    return left.nonzero_forces_const(target, context);
                }
                None
            }
            SymExprInner::Op(SymExprOp::Shl | SymExprOp::Shr, value, shift)
                if shift.eval_const().is_some_and(|shift| shift.is_zero()) =>
            {
                value.nonzero_forces_const(target, context)
            }
            SymExprInner::AddMod { .. } | SymExprInner::MulMod { .. } => None,
            SymExprInner::Op(_, _, _) => None,
        }
    }

    pub(crate) fn is_raw_gasleft(&self) -> bool {
        matches!(self.as_inner(), SymExprInner::GasLeft(_))
    }

    pub(crate) fn var(name: &str) -> Self {
        Self::var_symbol(Symbol::intern(name))
    }

    pub(crate) fn var_symbol(name: Symbol) -> Self {
        Self::from_inner(SymExprInner::Var(name))
    }

    pub(crate) fn gas_left(id: usize) -> Self {
        Self::from_inner(SymExprInner::GasLeft(id))
    }

    pub(crate) fn keccak(name: &str, len: Self, bytes: Vec<Self>) -> Self {
        Self::keccak_symbol(Symbol::intern(name), len, bytes)
    }

    pub(crate) fn keccak_symbol(name: Symbol, len: Self, bytes: Vec<Self>) -> Self {
        Self::from_inner(SymExprInner::Keccak { name, len, bytes: bytes.into() })
    }

    pub(crate) fn hash(name: &str, algorithm: &'static str, bytes: Vec<Self>) -> Self {
        Self::hash_symbol(Symbol::intern(name), algorithm, bytes)
    }

    pub(crate) fn hash_symbol(name: Symbol, algorithm: &'static str, bytes: Vec<Self>) -> Self {
        Self::from_inner(SymExprInner::Hash { name, algorithm, bytes: bytes.into() })
    }

    pub(crate) fn ite(cond: SymBoolExpr, then_expr: Self, else_expr: Self) -> Self {
        match cond.into_inner() {
            SymBoolExprInner::Const(true) => then_expr,
            SymBoolExprInner::Const(false) => else_expr,
            _ if then_expr == else_expr => then_expr,
            cond => Self::from_inner(SymExprInner::Ite(
                SymBoolExpr::from_inner(cond),
                then_expr,
                else_expr,
            )),
        }
    }

    pub(crate) fn add_const(expr: Self, value: U256) -> Self {
        if value.is_zero() {
            return expr;
        }
        match expr.into_inner() {
            SymExprInner::Const(expr) => Self::constant(expr.wrapping_add(value)),
            expr => Self::from_inner(SymExprInner::Op(
                SymExprOp::Add,
                Self::from_inner(expr),
                Self::constant(value),
            )),
        }
    }

    pub(crate) fn not(value: Self) -> Self {
        match value.into_inner() {
            SymExprInner::Const(value) => Self::constant(!value),
            SymExprInner::Not(value) => value,
            value => Self::from_inner(SymExprInner::Not(Self::from_inner(value))),
        }
    }

    /// Visits this expression and all child expressions.
    pub(crate) fn visit<B>(
        &self,
        visitor: &mut impl FnMut(&Self) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        visitor(self)?;
        match self.as_inner() {
            SymExprInner::Const(_) | SymExprInner::Var(_) | SymExprInner::GasLeft(_) => {}
            SymExprInner::Keccak { len, bytes, .. } => {
                len.visit(visitor)?;
                for byte in bytes.iter() {
                    byte.visit(visitor)?;
                }
            }
            SymExprInner::Hash { bytes, .. } => {
                for byte in bytes.iter() {
                    byte.visit(visitor)?;
                }
            }
            SymExprInner::Not(value) => value.visit(visitor)?,
            SymExprInner::Op(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
            SymExprInner::AddMod { left, right, modulus }
            | SymExprInner::MulMod { left, right, modulus } => {
                left.visit(visitor)?;
                right.visit(visitor)?;
                modulus.visit(visitor)?;
            }
            SymExprInner::Ite(cond, left, right) => {
                cond.visit_exprs(visitor)?;
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn op(op: SymExprOp, left: Self, right: Self) -> Self {
        match (op, left.into_inner(), right.into_inner()) {
            (op, SymExprInner::Const(left), SymExprInner::Const(right)) => {
                Self::constant(op.eval(left, right))
            }
            (SymExprOp::Add, SymExprInner::Const(value), right) if value.is_zero() => {
                Self::from_inner(right)
            }
            (SymExprOp::Add, left, SymExprInner::Const(value)) if value.is_zero() => {
                Self::from_inner(left)
            }
            (SymExprOp::Sub, left, SymExprInner::Const(value)) if value.is_zero() => {
                Self::from_inner(left)
            }
            (SymExprOp::Sub, left, right) if left == right => Self::constant(U256::ZERO),
            (SymExprOp::Mul, SymExprInner::Const(value), _)
            | (SymExprOp::Mul, _, SymExprInner::Const(value))
                if value.is_zero() =>
            {
                Self::constant(U256::ZERO)
            }
            (SymExprOp::Mul, SymExprInner::Const(value), right) if value == U256::from(1) => {
                Self::from_inner(right)
            }
            (SymExprOp::Mul, left, SymExprInner::Const(value)) if value == U256::from(1) => {
                Self::from_inner(left)
            }
            (
                SymExprOp::UDiv | SymExprOp::URem | SymExprOp::SDiv | SymExprOp::SRem,
                _,
                SymExprInner::Const(value),
            ) if value.is_zero() => Self::constant(U256::ZERO),
            (SymExprOp::UDiv | SymExprOp::SDiv, left, SymExprInner::Const(value))
                if value == U256::from(1) =>
            {
                Self::from_inner(left)
            }
            (SymExprOp::URem | SymExprOp::SRem, _, SymExprInner::Const(value))
                if value == U256::from(1) =>
            {
                Self::constant(U256::ZERO)
            }
            (SymExprOp::And, SymExprInner::Const(value), _)
            | (SymExprOp::And, _, SymExprInner::Const(value))
                if value.is_zero() =>
            {
                Self::constant(U256::ZERO)
            }
            (SymExprOp::And, SymExprInner::Const(value), right) if value == U256::MAX => {
                Self::from_inner(right)
            }
            (SymExprOp::And, left, SymExprInner::Const(value)) if value == U256::MAX => {
                Self::from_inner(left)
            }
            (SymExprOp::And, left, right) if left == right => Self::from_inner(left),
            (SymExprOp::And, SymExprInner::Const(mask), right) => {
                Self::and_const(Self::from_inner(right), mask)
            }
            (SymExprOp::And, left, SymExprInner::Const(mask)) => {
                Self::and_const(Self::from_inner(left), mask)
            }
            (SymExprOp::Or | SymExprOp::Xor, SymExprInner::Const(value), right)
                if value.is_zero() =>
            {
                Self::from_inner(right)
            }
            (SymExprOp::Or | SymExprOp::Xor, left, SymExprInner::Const(value))
                if value.is_zero() =>
            {
                Self::from_inner(left)
            }
            (
                SymExprOp::Shl | SymExprOp::Shr | SymExprOp::Sar,
                left,
                SymExprInner::Const(value),
            ) if value.is_zero() => Self::from_inner(left),
            (SymExprOp::Shl | SymExprOp::Shr, SymExprInner::Const(value), _) if value.is_zero() => {
                Self::constant(U256::ZERO)
            }
            (op, left, right) => Self::from_inner(SymExprInner::Op(
                op,
                Self::from_inner(left),
                Self::from_inner(right),
            )),
        }
    }

    /// Builds an exact EVM `ADDMOD` expression.
    pub(crate) fn addmod(left: Self, right: Self, modulus: Self) -> Self {
        match (left.into_inner(), right.into_inner(), modulus.into_inner()) {
            (_, _, SymExprInner::Const(modulus))
                if modulus.is_zero() || modulus == U256::from(1) =>
            {
                Self::constant(U256::ZERO)
            }
            (
                SymExprInner::Const(left),
                SymExprInner::Const(right),
                SymExprInner::Const(modulus),
            ) => Self::constant(addmod_word(left, right, modulus)),
            (left, right, modulus) => Self::from_inner(SymExprInner::AddMod {
                left: Self::from_inner(left),
                right: Self::from_inner(right),
                modulus: Self::from_inner(modulus),
            }),
        }
    }

    /// Builds an exact EVM `MULMOD` expression.
    pub(crate) fn mulmod(left: Self, right: Self, modulus: Self) -> Self {
        match (left.into_inner(), right.into_inner(), modulus.into_inner()) {
            (_, _, SymExprInner::Const(modulus))
                if modulus.is_zero() || modulus == U256::from(1) =>
            {
                Self::constant(U256::ZERO)
            }
            (
                SymExprInner::Const(left),
                SymExprInner::Const(right),
                SymExprInner::Const(modulus),
            ) => Self::constant(mulmod_word(left, right, modulus)),
            (left, right, modulus) => Self::from_inner(SymExprInner::MulMod {
                left: Self::from_inner(left),
                right: Self::from_inner(right),
                modulus: Self::from_inner(modulus),
            }),
        }
    }

    fn and_const(expr: Self, mask: U256) -> Self {
        if mask.is_zero() {
            return Self::constant(U256::ZERO);
        }
        if mask == U256::MAX {
            return expr;
        }

        match expr.into_inner() {
            SymExprInner::Op(SymExprOp::And, left, right) => {
                match (left.into_inner(), right.into_inner()) {
                    (SymExprInner::Const(value), right) if value == mask => {
                        Self::and_const(Self::from_inner(right), mask)
                    }
                    (left, SymExprInner::Const(value)) if value == mask => {
                        Self::and_const(Self::from_inner(left), mask)
                    }
                    (left, right) if left == right => Self::and_const(Self::from_inner(left), mask),
                    (left, right) => Self::from_inner(SymExprInner::Op(
                        SymExprOp::And,
                        Self::from_inner(SymExprInner::Op(
                            SymExprOp::And,
                            Self::from_inner(left),
                            Self::from_inner(right),
                        )),
                        Self::constant(mask),
                    )),
                }
            }
            expr => Self::from_inner(SymExprInner::Op(
                SymExprOp::And,
                Self::from_inner(expr),
                Self::constant(mask),
            )),
        }
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit(&mut |expr| {
            match expr.as_inner() {
                SymExprInner::Var(var) => {
                    vars.insert(*var);
                }
                SymExprInner::Keccak { name, .. } => {
                    vars.insert(*name);
                }
                SymExprInner::Hash { name, .. } => {
                    vars.insert(*name);
                }
                SymExprInner::Const(_)
                | SymExprInner::GasLeft(_)
                | SymExprInner::Not(_)
                | SymExprInner::Op(_, _, _)
                | SymExprInner::AddMod { .. }
                | SymExprInner::MulMod { .. }
                | SymExprInner::Ite(_, _, _) => {}
            }
            ControlFlow::<()>::Continue(())
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
            SymExprInner::Const(value) => {
                let _ = write!(out, "(_ bv{value} 256)");
            }
            SymExprInner::Var(var) => out.push_str(var.as_str()),
            SymExprInner::GasLeft(id) => {
                let _ = write!(out, "gasleft_{id}");
            }
            SymExprInner::Keccak { name, .. } => out.push_str(name.as_str()),
            SymExprInner::Hash { name, .. } => out.push_str(name.as_str()),
            SymExprInner::Not(value) => {
                out.push_str("(bvnot ");
                value.write_smt(out);
                out.push(')');
            }
            SymExprInner::Op(op, left, right) => {
                let _ = write!(out, "({} ", op.smt());
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymExprInner::AddMod { left, right, modulus } => {
                write_smt_wide_modular_arithmetic(out, "bvadd", left, right, modulus);
            }
            SymExprInner::MulMod { left, right, modulus } => {
                write_smt_wide_modular_arithmetic(out, "bvmul", left, right, modulus);
            }
            SymExprInner::Ite(cond, left, right) => {
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
    left: &SymExpr,
    right: &SymExpr,
    modulus: &SymExpr,
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
pub(crate) struct SymBoolExpr(Arc<SymBoolExprInner>);

impl fmt::Debug for SymBoolExpr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.as_inner().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) enum SymBoolExprInner {
    Const(bool),
    Not(SymBoolExpr),
    And(Arc<[SymBoolExpr]>),
    Eq(SymExpr, SymExpr),
    Cmp(SymBoolExprOp, SymExpr, SymExpr),
}

static BOOL_TRUE: LazyLock<Arc<SymBoolExprInner>> =
    LazyLock::new(|| Arc::new(SymBoolExprInner::Const(true)));
static BOOL_FALSE: LazyLock<Arc<SymBoolExprInner>> =
    LazyLock::new(|| Arc::new(SymBoolExprInner::Const(false)));

impl SymBoolExpr {
    fn from_inner(expr: SymBoolExprInner) -> Self {
        match expr {
            SymBoolExprInner::Const(value) => Self::constant(value),
            expr => Self(Arc::new(expr)),
        }
    }

    pub(crate) fn constant(value: bool) -> Self {
        Self(if value { BOOL_TRUE.clone() } else { BOOL_FALSE.clone() })
    }

    pub(super) fn as_inner(&self) -> &SymBoolExprInner {
        self.0.as_ref()
    }

    pub(super) fn into_inner(self) -> SymBoolExprInner {
        Arc::unwrap_or_clone(self.0)
    }

    pub(crate) fn as_const(&self) -> Option<bool> {
        match self.as_inner() {
            SymBoolExprInner::Const(value) => Some(*value),
            _ => None,
        }
    }

    pub(crate) fn eval_const(&self) -> Option<bool> {
        match self.as_inner() {
            SymBoolExprInner::Const(value) => Some(*value),
            SymBoolExprInner::Not(value) => Some(!value.eval_const()?),
            SymBoolExprInner::And(values) => {
                let mut all_true = true;
                for value in values.iter() {
                    all_true &= value.eval_const()?;
                }
                Some(all_true)
            }
            SymBoolExprInner::Eq(left, right) => Some(left.eval_const()? == right.eval_const()?),
            SymBoolExprInner::Cmp(op, left, right) => {
                Some(op.eval(left.eval_const()?, right.eval_const()?))
            }
        }
    }

    pub(crate) fn contains_keccak(&self) -> bool {
        self.visit_exprs(&mut |expr| {
            if matches!(expr.as_inner(), SymExprInner::Keccak { .. }) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        self.visit_exprs(&mut |expr| {
            if matches!(expr.as_inner(), SymExprInner::GasLeft(_)) {
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .is_break()
    }

    pub(crate) fn forces_expr_const_with_context(
        &self,
        expr: &SymExpr,
        context: &[Self],
    ) -> Option<U256> {
        match self.as_inner() {
            SymBoolExprInner::Eq(left, right) => match (left.as_inner(), right.as_inner()) {
                (_, SymExprInner::Const(value)) => {
                    left.equality_forces_const(*value, expr, context)
                }
                (SymExprInner::Const(value), _) => {
                    right.equality_forces_const(*value, expr, context)
                }
                _ => None,
            },
            SymBoolExprInner::Not(value) => match value.as_inner() {
                SymBoolExprInner::Eq(left, right) => match (left.as_inner(), right.as_inner()) {
                    (_, SymExprInner::Const(value)) if value.is_zero() => {
                        left.nonzero_forces_const(expr, context)
                    }
                    (SymExprInner::Const(value), _) if value.is_zero() => {
                        right.nonzero_forces_const(expr, context)
                    }
                    _ => None,
                },
                SymBoolExprInner::Not(value) => value.forces_expr_const_with_context(expr, context),
                _ => None,
            },
            SymBoolExprInner::And(values) => {
                values.iter().find_map(|value| value.forces_expr_const_with_context(expr, context))
            }
            _ => None,
        }
    }

    pub(crate) fn upper_bound_usize(&self, expr: &SymExpr) -> Option<usize> {
        match self.as_inner() {
            SymBoolExprInner::Const(_) | SymBoolExprInner::Not(_) => None,
            SymBoolExprInner::And(values) => {
                let mut bound: Option<usize> = None;
                for value in values.iter() {
                    if let Some(candidate) = value.upper_bound_usize(expr) {
                        bound = Some(bound.map_or(candidate, |bound| bound.min(candidate)));
                    }
                }
                bound
            }
            SymBoolExprInner::Eq(left, right) => match (left == expr, right == expr) {
                (true, _) => right.eval_const().and_then(|value| usize::try_from(value).ok()),
                (_, true) => left.eval_const().and_then(|value| usize::try_from(value).ok()),
                _ => None,
            },
            SymBoolExprInner::Cmp(op, left, right) => {
                if left == expr {
                    match *op {
                        SymBoolExprOp::Ult => right
                            .eval_const()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Ule => {
                            right.eval_const().and_then(|value| usize::try_from(value).ok())
                        }
                        _ => None,
                    }
                } else if right == expr {
                    match *op {
                        SymBoolExprOp::Ugt => left
                            .eval_const()
                            .and_then(|bound| (!bound.is_zero()).then(|| bound - U256::from(1)))
                            .and_then(|value| usize::try_from(value).ok()),
                        SymBoolExprOp::Uge => {
                            left.eval_const().and_then(|value| usize::try_from(value).ok())
                        }
                        _ => None,
                    }
                } else {
                    None
                }
            }
        }
    }

    pub(crate) fn eval<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<bool, SymbolicError> {
        Ok(match self.as_inner() {
            SymBoolExprInner::Const(value) => *value,
            SymBoolExprInner::Not(value) => !value.eval(model)?,
            SymBoolExprInner::And(values) => {
                for value in values.iter() {
                    if !value.eval(model)? {
                        return Ok(false);
                    }
                }
                true
            }
            SymBoolExprInner::Eq(left, right) => left.eval(model)? == right.eval(model)?,
            SymBoolExprInner::Cmp(op, left, right) => {
                op.eval(left.eval(model)?, right.eval(model)?)
            }
        })
    }

    /// Visits this boolean expression and all child boolean expressions.
    pub(crate) fn visit<B>(
        &self,
        visitor: &mut impl FnMut(&Self) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        visitor(self)?;
        match self.as_inner() {
            SymBoolExprInner::Const(_)
            | SymBoolExprInner::Eq(_, _)
            | SymBoolExprInner::Cmp(_, _, _) => {}
            SymBoolExprInner::Not(value) => value.visit(visitor)?,
            SymBoolExprInner::And(values) => {
                for value in values.iter() {
                    value.visit(visitor)?;
                }
            }
        }
        ControlFlow::Continue(())
    }

    /// Visits all word expressions contained in this boolean expression.
    pub(crate) fn visit_exprs<B>(
        &self,
        visitor: &mut impl FnMut(&SymExpr) -> ControlFlow<B>,
    ) -> ControlFlow<B> {
        match self.as_inner() {
            SymBoolExprInner::Const(_) => {}
            SymBoolExprInner::Not(value) => value.visit_exprs(visitor)?,
            SymBoolExprInner::And(values) => {
                for value in values.iter() {
                    value.visit_exprs(visitor)?;
                }
            }
            SymBoolExprInner::Eq(left, right) | SymBoolExprInner::Cmp(_, left, right) => {
                left.visit(visitor)?;
                right.visit(visitor)?;
            }
        }
        ControlFlow::Continue(())
    }

    pub(crate) fn eq(left: SymExpr, right: SymExpr) -> Self {
        match (left.into_inner(), right.into_inner()) {
            (left, right) if left == right => Self::constant(true),
            (SymExprInner::Const(left), SymExprInner::Const(right)) => {
                Self::constant(left == right)
            }
            (left, SymExprInner::Const(right_value)) => {
                let left = SymExpr::from_inner(left);
                if let Some(left_value) = left.known_word() {
                    return Self::constant(left_value == right_value);
                }
                Self::from_inner(SymBoolExprInner::Eq(left, SymExpr::constant(right_value)))
            }
            (SymExprInner::Const(left_value), right) => {
                let right = SymExpr::from_inner(right);
                if let Some(right_value) = right.known_word() {
                    return Self::constant(left_value == right_value);
                }
                Self::from_inner(SymBoolExprInner::Eq(SymExpr::constant(left_value), right))
            }
            (
                SymExprInner::Keccak { len: left_len, bytes: left_bytes, .. },
                SymExprInner::Keccak { len: right_len, bytes: right_bytes, .. },
            ) if left_bytes.len() == right_bytes.len() => {
                let mut conditions = vec![Self::eq(left_len, right_len)];
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
                SymExprInner::Hash { algorithm: left_algorithm, bytes: left_bytes, .. },
                SymExprInner::Hash { algorithm: right_algorithm, bytes: right_bytes, .. },
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
            (left, right) => Self::from_inner(SymBoolExprInner::Eq(
                SymExpr::from_inner(left),
                SymExpr::from_inner(right),
            )),
        }
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
            match value.into_inner() {
                SymBoolExprInner::Const(true) => {}
                SymBoolExprInner::Const(false) => return Self::constant(false),
                SymBoolExprInner::And(values) => out.extend(values.iter().cloned()),
                value => out.push(Self::from_inner(value)),
            }
        }
        if out.is_empty() {
            Self::constant(true)
        } else if out.len() == 1 {
            out.pop().expect("single item exists")
        } else {
            Self::from_inner(SymBoolExprInner::And(out.into()))
        }
    }

    #[cfg(test)]
    pub(crate) fn raw_and(values: Vec<Self>) -> Self {
        Self::from_inner(SymBoolExprInner::And(values.into()))
    }

    pub(crate) fn or(values: Vec<Self>) -> Self {
        let mut out = Vec::new();
        for value in values {
            match value.into_inner() {
                SymBoolExprInner::Const(false) => {}
                SymBoolExprInner::Const(true) => return Self::constant(true),
                value => out.push(Self::from_inner(value)),
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
        match (op, left.into_inner(), right.into_inner()) {
            (op, left, right) if left == right => {
                Self::constant(matches!(op, SymBoolExprOp::Ule | SymBoolExprOp::Uge))
            }
            (op, SymExprInner::Const(left), SymExprInner::Const(right)) => {
                Self::constant(op.eval(left, right))
            }
            (SymBoolExprOp::Ugt, SymExprInner::Const(value), _) if value.is_zero() => {
                Self::constant(false)
            }
            (SymBoolExprOp::Ule, SymExprInner::Const(value), _) if value.is_zero() => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ult, _, SymExprInner::Const(value)) if value.is_zero() => {
                Self::constant(false)
            }
            (SymBoolExprOp::Uge, _, SymExprInner::Const(value)) if value.is_zero() => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ult, SymExprInner::Const(value), _) if value == U256::MAX => {
                Self::constant(false)
            }
            (SymBoolExprOp::Uge, SymExprInner::Const(value), _) if value == U256::MAX => {
                Self::constant(true)
            }
            (SymBoolExprOp::Ugt, _, SymExprInner::Const(value)) if value == U256::MAX => {
                Self::constant(false)
            }
            (SymBoolExprOp::Ule, _, SymExprInner::Const(value)) if value == U256::MAX => {
                Self::constant(true)
            }
            (op, left, right) => Self::from_inner(SymBoolExprInner::Cmp(
                op,
                SymExpr::from_inner(left),
                SymExpr::from_inner(right),
            )),
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
        match self.into_inner() {
            SymBoolExprInner::Const(value) => Self::constant(!value),
            SymBoolExprInner::Not(value) => value,
            value => Self::from_inner(SymBoolExprInner::Not(Self::from_inner(value))),
        }
    }

    pub(crate) fn collect_vars(&self, vars: &mut SymbolicVars) {
        let _ = self.visit(&mut |expr| {
            match expr.as_inner() {
                SymBoolExprInner::Eq(left, right) | SymBoolExprInner::Cmp(_, left, right) => {
                    left.collect_vars(vars);
                    right.collect_vars(vars);
                }
                SymBoolExprInner::Const(_)
                | SymBoolExprInner::Not(_)
                | SymBoolExprInner::And(_) => {}
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
        match self.as_inner() {
            SymBoolExprInner::Const(value) => out.push_str(if *value { "true" } else { "false" }),
            SymBoolExprInner::Not(value) => {
                out.push_str("(not ");
                value.write_smt(out);
                out.push(')');
            }
            SymBoolExprInner::And(values) => {
                out.push_str("(and");
                for value in values.iter() {
                    out.push(' ');
                    value.write_smt(out);
                }
                out.push(')');
            }
            SymBoolExprInner::Eq(left, right) => {
                out.push_str("(= ");
                left.write_smt(out);
                out.push(' ');
                right.write_smt(out);
                out.push(')');
            }
            SymBoolExprInner::Cmp(op, left, right) => {
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
