use super::*;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymBytes(Arc<SymBytesKind>);

impl fmt::Debug for SymBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SymBytesKind {
    Concrete(Arc<[u8]>),
    Exprs(Arc<[SymExpr]>),
    Word(SymExpr),
    Concat(Arc<[SymBytes]>),
    Slice { bytes: SymBytes, offset: SymExpr, len: usize },
    Sized { bytes: SymBytes, size: SymExpr, max_size: usize },
}

static BYTES_EMPTY: LazyLock<Arc<SymBytesKind>> =
    LazyLock::new(|| Arc::new(SymBytesKind::Concrete(Vec::<u8>::new().into())));

impl Default for SymBytes {
    fn default() -> Self {
        Self(BYTES_EMPTY.clone())
    }
}

impl SymBytes {
    fn from_kind(kind: SymBytesKind) -> Self {
        match kind {
            SymBytesKind::Concrete(bytes) if bytes.is_empty() => Self::default(),
            kind => Self(Arc::new(kind)),
        }
    }

    fn kind(&self) -> &SymBytesKind {
        self.0.as_ref()
    }

    pub(crate) fn concrete(bytes: Vec<u8>) -> Self {
        Self::from_kind(SymBytesKind::Concrete(bytes.into()))
    }

    pub(crate) fn exprs(bytes: Vec<SymExpr>) -> Self {
        if let Ok(concrete) = bytes.concrete_bytes("symbolic bytes") {
            Self::concrete(concrete)
        } else {
            Self::from_kind(SymBytesKind::Exprs(bytes.into()))
        }
    }

    pub(crate) fn from_shared_exprs(bytes: Arc<[SymExpr]>) -> Self {
        if let Ok(concrete) = bytes.concrete_bytes("symbolic bytes") {
            Self::concrete(concrete)
        } else {
            Self::from_kind(SymBytesKind::Exprs(bytes))
        }
    }

    pub(crate) fn word(word: SymExpr) -> Self {
        if let Some(word) = word.as_const() {
            Self::concrete(word.to_be_bytes::<32>().to_vec())
        } else {
            Self::from_kind(SymBytesKind::Word(word))
        }
    }

    pub(crate) fn concat(bytes: impl IntoIterator<Item = Self>) -> Self {
        let mut out = Vec::new();
        for bytes in bytes {
            match bytes.kind() {
                SymBytesKind::Concrete(values) if values.is_empty() => {}
                SymBytesKind::Concat(values) => out.extend(values.iter().cloned()),
                _ => out.push(bytes),
            }
        }
        match out.len() {
            0 => Self::default(),
            1 => out.pop().expect("single item exists"),
            _ => Self::from_kind(SymBytesKind::Concat(out.into())),
        }
    }

    pub(crate) fn slice(bytes: Self, offset: SymExpr, len: usize) -> Self {
        if len == 0 {
            return Self::default();
        }
        if let Some(offset) = offset.eval() {
            let Ok(offset) = usize::try_from(offset) else {
                return Self::concrete(vec![0; len]);
            };
            return bytes.slice_concrete(offset, len);
        }
        Self::from_kind(SymBytesKind::Slice { bytes, offset, len })
    }

    pub(crate) fn sized(bytes: Self, size: SymExpr, max_size: usize) -> Self {
        if max_size == 0 {
            return Self::default();
        }
        if let Some(size) = size.eval() {
            let size = usize::try_from(size).map_or(max_size, |size| size.min(max_size));
            let bytes = bytes.slice_concrete(0, size);
            return Self::concat([bytes, Self::concrete(vec![0; max_size - size])]);
        }
        Self::from_kind(SymBytesKind::Sized { bytes, size, max_size })
    }

    pub(crate) fn len(&self) -> usize {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => bytes.len(),
            SymBytesKind::Exprs(bytes) => bytes.len(),
            SymBytesKind::Word(_) => 32,
            SymBytesKind::Concat(values) => {
                values.iter().fold(0usize, |len, bytes| len.saturating_add(bytes.len()))
            }
            SymBytesKind::Slice { len, .. } => *len,
            SymBytesKind::Sized { max_size, .. } => *max_size,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn byte(&self, offset: usize) -> SymExpr {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => bytes
                .get(offset)
                .copied()
                .map(|byte| SymExpr::constant(U256::from(byte)))
                .unwrap_or_else(SymExpr::zero),
            SymBytesKind::Exprs(bytes) => bytes.get(offset).cloned().unwrap_or_else(SymExpr::zero),
            SymBytesKind::Word(word) => {
                if offset >= 32 {
                    SymExpr::zero()
                } else if let Some(byte) = word.known_byte(offset) {
                    SymExpr::constant(U256::from(byte))
                } else {
                    word.extracted_byte(offset)
                }
            }
            SymBytesKind::Concat(values) => {
                let mut offset = offset;
                for bytes in values.iter() {
                    if offset < bytes.len() {
                        return bytes.byte(offset);
                    }
                    offset = offset.saturating_sub(bytes.len());
                }
                SymExpr::zero()
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len } => {
                if offset >= *len {
                    SymExpr::zero()
                } else {
                    bytes.byte_dynamic_with_delta(base_offset, offset)
                }
            }
            SymBytesKind::Sized { bytes, size, max_size } => {
                if offset >= *max_size {
                    return SymExpr::zero();
                }
                let source = bytes.byte(offset);
                SymExpr::ite(
                    SymBoolExpr::cmp(
                        SymBoolExprOp::Ult,
                        SymExpr::constant(U256::from(offset)),
                        size.clone(),
                    ),
                    source,
                    SymExpr::zero(),
                )
            }
        }
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::zero();
        for candidate in (delta..self.len()).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                self.byte(candidate),
                result,
            );
        }
        result
    }

    pub(crate) fn slice_concrete(&self, offset: usize, len: usize) -> Self {
        if len == 0 {
            return Self::default();
        }
        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let out = (0..len)
                    .map(|idx| bytes.get(offset + idx).copied().unwrap_or_default())
                    .collect();
                Self::concrete(out)
            }
            SymBytesKind::Exprs(bytes) => Self::exprs(
                (0..len)
                    .map(|idx| bytes.get(offset + idx).cloned().unwrap_or_else(SymExpr::zero))
                    .collect(),
            ),
            SymBytesKind::Word(word) if offset == 0 && len == 32 => Self::word(word.clone()),
            _ => Self::exprs((0..len).map(|idx| self.byte(offset + idx)).collect()),
        }
    }

    pub(crate) fn read_offset(&self, offset: SymExpr, len: usize) -> Self {
        Self::slice(self.clone(), offset, len)
    }

    pub(crate) fn word_at(&self, offset: usize) -> SymExpr {
        match self.kind() {
            SymBytesKind::Word(word) if offset == 0 => word.clone(),
            SymBytesKind::Slice { bytes, offset: base_offset, len }
                if offset == 0 && *len >= 32 =>
            {
                if let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                {
                    return bytes.word_at(base_offset);
                }
                SymExpr::from_bytes(
                    (0..32).map(|idx| bytes.byte_dynamic_with_delta(base_offset, idx)),
                )
            }
            _ => SymExpr::from_bytes((0..32).map(|idx| self.byte(offset + idx))),
        }
    }

    pub(crate) fn materialize(&self) -> Vec<SymExpr> {
        (0..self.len()).map(|idx| self.byte(idx)).collect()
    }

    pub(crate) fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Ok(bytes.to_vec()),
            _ => self.materialize().concrete_bytes(reason),
        }
    }
}
