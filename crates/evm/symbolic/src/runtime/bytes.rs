use super::*;

#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct SymBytes(Arc<SymBytesKind>);

impl fmt::Debug for SymBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum SymBytesKind {
    Concrete(Vec<u8>),
    Exprs(Vec<SymExpr>),
    Word(SymExpr),
    Concat(Vec<SymBytes>),
    Slice { bytes: SymBytes, offset: SymExpr, len: usize },
    Sized { bytes: SymBytes, size: SymExpr, max_size: usize },
}

static BYTES_EMPTY: LazyLock<Arc<SymBytesKind>> =
    LazyLock::new(|| Arc::new(SymBytesKind::Concrete(Vec::new())));

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
        Self::from_kind(SymBytesKind::Concrete(bytes))
    }

    pub(crate) fn as_concrete_slice(&self) -> Option<&[u8]> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn exprs(bytes: Vec<SymExpr>) -> Self {
        if let Ok(concrete) = concrete_expr_bytes(&bytes, "symbolic bytes") {
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
            _ => Self::from_kind(SymBytesKind::Concat(out)),
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
        Self::slice_node(bytes, offset, len)
    }

    fn slice_node(bytes: Self, offset: SymExpr, len: usize) -> Self {
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
                for bytes in values {
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
                } else if let Some(base_offset) = base_offset.eval() {
                    let Ok(base_offset) = usize::try_from(base_offset) else {
                        return SymExpr::zero();
                    };
                    let Some(offset) = base_offset.checked_add(offset) else {
                        return SymExpr::zero();
                    };
                    bytes.byte(offset)
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
        if offset == 0 && len == self.len() {
            return self.clone();
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
            SymBytesKind::Word(_) | SymBytesKind::Sized { .. } => {
                Self::slice_node(self.clone(), SymExpr::constant(U256::from(offset)), len)
            }
            SymBytesKind::Slice { bytes, offset: base_offset, .. } => {
                if let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(offset) = base_offset.checked_add(offset)
                {
                    bytes.slice_concrete(offset, len)
                } else {
                    Self::slice_node(self.clone(), SymExpr::constant(U256::from(offset)), len)
                }
            }
            SymBytesKind::Concat(values) => {
                let mut offset = offset;
                let mut len = len;
                let mut out = Vec::new();
                for bytes in values {
                    if len == 0 {
                        break;
                    }

                    let bytes_len = bytes.len();
                    if offset >= bytes_len {
                        offset -= bytes_len;
                        continue;
                    }

                    let take = (bytes_len - offset).min(len);
                    if offset == 0 && take == bytes_len {
                        out.push(bytes.clone());
                    } else {
                        out.push(bytes.slice_concrete(offset, take));
                    }
                    len -= take;
                    offset = 0;
                }

                if len != 0 {
                    out.push(Self::concrete(vec![0; len]));
                }
                Self::concat(out)
            }
        }
    }

    pub(crate) fn read_offset(&self, offset: SymExpr, len: usize) -> Self {
        Self::slice(self.clone(), offset, len)
    }

    pub(crate) fn word_at(&self, offset: usize) -> SymExpr {
        if let Some(word) = self.word_fragment_at(offset, 32, 0) {
            return word;
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let mut word = [0u8; 32];
                if offset < bytes.len() {
                    let take = (bytes.len() - offset).min(32);
                    word[..take].copy_from_slice(&bytes[offset..offset + take]);
                }
                SymExpr::constant(U256::from_be_bytes(word))
            }
            SymBytesKind::Exprs(bytes) => SymExpr::from_bytes(
                (0..32).map(|idx| bytes.get(offset + idx).cloned().unwrap_or_else(SymExpr::zero)),
            ),
            SymBytesKind::Word(word) if offset == 0 => word.clone(),
            SymBytesKind::Word(_) if offset >= 32 => SymExpr::zero(),
            SymBytesKind::Slice { bytes, offset: base_offset, len } => {
                if offset.checked_add(32).is_some_and(|end| end <= *len)
                    && let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(base_offset) = base_offset.checked_add(offset)
                {
                    return bytes.word_at(base_offset);
                }
                SymExpr::from_bytes((0..32).map(|idx| self.byte(offset + idx)))
            }
            _ => SymExpr::from_bytes((0..32).map(|idx| self.byte(offset + idx))),
        }
    }

    pub(crate) fn right_aligned_word(&self, offset: usize, len: usize) -> SymExpr {
        debug_assert!(len <= 32);
        let len = len.min(32);
        if let Some(word) = self.word_fragment_at(offset, len, 32 - len) {
            return word;
        }

        SymExpr::from_bytes(
            std::iter::repeat_with(SymExpr::zero)
                .take(32 - len)
                .chain((0..len).map(|idx| self.byte(offset + idx))),
        )
    }

    fn word_fragment_at(&self, offset: usize, len: usize, out_offset: usize) -> Option<SymExpr> {
        debug_assert!(len <= 32);
        debug_assert!(out_offset <= 32);
        debug_assert!(out_offset + len <= 32);

        if len == 0 {
            return Some(SymExpr::zero());
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let mut word = [0u8; 32];
                if offset < bytes.len() {
                    let take = (bytes.len() - offset).min(len);
                    word[out_offset..out_offset + take]
                        .copy_from_slice(&bytes[offset..offset + take]);
                }
                Some(SymExpr::constant(U256::from_be_bytes(word)))
            }
            SymBytesKind::Word(word) => {
                Self::word_expr_fragment(word.clone(), offset, len, out_offset)
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len: slice_len } => {
                let available = slice_len.saturating_sub(offset).min(len);
                if available == 0 {
                    return Some(SymExpr::zero());
                }
                let base_offset =
                    base_offset.eval().and_then(|offset| usize::try_from(offset).ok())?;
                bytes.word_fragment_at(base_offset.checked_add(offset)?, available, out_offset)
            }
            SymBytesKind::Concat(values) => {
                let mut offset = offset;
                let mut out_offset = out_offset;
                let mut remaining = len;
                let mut out = SymExpr::zero();

                for bytes in values {
                    if remaining == 0 {
                        break;
                    }

                    let bytes_len = bytes.len();
                    if offset >= bytes_len {
                        offset -= bytes_len;
                        continue;
                    }

                    let take = (bytes_len - offset).min(remaining);
                    out = SymExpr::op(
                        SymExprOp::Or,
                        out,
                        bytes.word_fragment_at(offset, take, out_offset)?,
                    );
                    out_offset += take;
                    remaining -= take;
                    offset = 0;
                }

                Some(out)
            }
            SymBytesKind::Exprs(_) | SymBytesKind::Sized { .. } => None,
        }
    }

    fn word_expr_fragment(
        word: SymExpr,
        offset: usize,
        len: usize,
        out_offset: usize,
    ) -> Option<SymExpr> {
        let len = 32usize.checked_sub(offset)?.min(len);
        if len == 0 {
            return Some(SymExpr::zero());
        }
        if offset == 0 && len == 32 && out_offset == 0 {
            return Some(word);
        }

        let src_trailing_bits = (32 - (offset + len)) * 8;
        let dst_trailing_bits = (32 - (out_offset + len)) * 8;
        let mask = mask_bits(U256::MAX, len * 8);

        let expr =
            SymExpr::op(SymExprOp::Shr, word, SymExpr::constant(U256::from(src_trailing_bits)));
        let expr = SymExpr::op(SymExprOp::And, expr, SymExpr::constant(mask));
        Some(SymExpr::op(SymExprOp::Shl, expr, SymExpr::constant(U256::from(dst_trailing_bits))))
    }

    pub(crate) fn materialize(&self) -> Vec<SymExpr> {
        (0..self.len()).map(|idx| self.byte(idx)).collect()
    }

    pub(crate) fn contains_gasleft(&self) -> bool {
        match self.kind() {
            SymBytesKind::Concrete(_) => false,
            SymBytesKind::Exprs(bytes) => bytes.iter().any(SymExpr::contains_gasleft),
            SymBytesKind::Word(word) => word.contains_gasleft(),
            SymBytesKind::Concat(values) => values.iter().any(Self::contains_gasleft),
            SymBytesKind::Slice { offset, .. } if offset.contains_gasleft() => true,
            SymBytesKind::Sized { size, .. } if size.contains_gasleft() => true,
            SymBytesKind::Slice { .. } | SymBytesKind::Sized { .. } => {
                (0..self.len()).any(|idx| self.byte(idx).contains_gasleft())
            }
        }
    }

    pub(crate) fn same_bytes(&self, other: &Self) -> bool {
        self.len() == other.len() && (0..self.len()).all(|idx| self.byte(idx) == other.byte(idx))
    }

    pub(crate) fn prefix_condition(&self, prefix: &Self) -> Option<SymBoolExpr> {
        if prefix.len() > self.len() {
            return None;
        }
        let mut conditions = Vec::new();
        for idx in 0..prefix.len() {
            let actual = self.byte(idx);
            let expected = prefix.byte(idx);
            if actual == expected {
                continue;
            }
            match (actual.as_const(), expected.as_const()) {
                (Some(actual), Some(expected)) if actual.to::<u8>() == expected.to::<u8>() => {}
                (Some(_), Some(_)) => return None,
                _ => conditions.push(SymBoolExpr::eq_words(&actual, &expected)),
            }
        }
        Some(SymBoolExpr::and(conditions))
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        model: &M,
    ) -> Result<Vec<u8>, SymbolicError> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Ok(bytes.clone()),
            _ => (0..self.len())
                .map(|idx| Ok(self.byte(idx).eval_model(model)?.to::<u8>()))
                .collect(),
        }
    }

    pub(crate) fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Ok(bytes.clone()),
            SymBytesKind::Exprs(bytes) => concrete_expr_bytes(bytes, reason),
            SymBytesKind::Concat(values) => {
                let mut out = Vec::with_capacity(self.len());
                for bytes in values {
                    out.extend(bytes.concrete_bytes(reason)?);
                }
                Ok(out)
            }
            SymBytesKind::Slice { bytes, offset, len } => {
                if let Some(offset) = offset.eval()
                    && let Ok(offset) = usize::try_from(offset)
                {
                    bytes.slice_concrete(offset, *len).concrete_bytes(reason)
                } else {
                    concrete_expr_bytes(&self.materialize(), reason)
                }
            }
            _ => concrete_expr_bytes(&self.materialize(), reason),
        }
    }
}
