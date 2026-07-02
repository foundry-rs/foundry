use super::{expr::hashcons::HashConsed, *};

#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct SymBytes {
    pub(in crate::runtime) kind: HashConsed<SymBytesKind>,
}

impl fmt::Debug for SymBytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind().fmt(f)
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
pub(in crate::runtime) enum SymBytesKind {
    Concrete(Vec<u8>),
    Exprs(Vec<SymExpr>),
    Word(SymExpr),
    Concat(Vec<SymBytes>),
    Slice { bytes: SymBytes, offset: SymExpr, len: usize },
    Sized { bytes: SymBytes, size: SymExpr, max_size: usize },
}

impl SymBytes {
    fn from_kind(cx: &mut SymCx, kind: SymBytesKind) -> Self {
        cx.mk_bytes_kind(kind)
    }

    fn kind(&self) -> &SymBytesKind {
        self.kind.value()
    }

    pub(crate) fn empty(cx: &mut SymCx) -> Self {
        Self::from_kind(cx, SymBytesKind::Concrete(Vec::new()))
    }

    pub(crate) fn concrete(cx: &mut SymCx, bytes: Vec<u8>) -> Self {
        Self::from_kind(cx, SymBytesKind::Concrete(bytes))
    }

    pub(crate) fn as_concrete_slice(&self) -> Option<&[u8]> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Some(bytes),
            _ => None,
        }
    }

    pub(crate) fn exprs(cx: &mut SymCx, bytes: Vec<SymExpr>) -> Self {
        if let Ok(concrete) = concrete_expr_bytes(&bytes, "symbolic bytes") {
            Self::concrete(cx, concrete)
        } else {
            Self::from_kind(cx, SymBytesKind::Exprs(bytes))
        }
    }

    pub(crate) fn word(cx: &mut SymCx, word: SymExpr) -> Self {
        if let Some(word) = word.as_const() {
            Self::concrete(cx, word.to_be_bytes::<32>().to_vec())
        } else {
            Self::from_kind(cx, SymBytesKind::Word(word))
        }
    }

    pub(crate) fn concat(cx: &mut SymCx, bytes: impl IntoIterator<Item = Self>) -> Self {
        let mut out = Vec::new();
        for bytes in bytes {
            match bytes.kind() {
                SymBytesKind::Concrete(values) if values.is_empty() => {}
                SymBytesKind::Concat(values) => out.extend(values.iter().cloned()),
                _ => out.push(bytes),
            }
        }
        match out.len() {
            0 => Self::empty(cx),
            1 => out.pop().expect("single item exists"),
            _ => Self::from_kind(cx, SymBytesKind::Concat(out)),
        }
    }

    pub(crate) fn slice(cx: &mut SymCx, bytes: Self, offset: SymExpr, len: usize) -> Self {
        if len == 0 {
            return Self::empty(cx);
        }
        if let Some(offset) = offset.eval() {
            let Ok(offset) = usize::try_from(offset) else {
                return Self::concrete(cx, vec![0; len]);
            };
            return bytes.slice_concrete(cx, offset, len);
        }
        Self::slice_node(cx, bytes, offset, len)
    }

    fn slice_node(cx: &mut SymCx, bytes: Self, offset: SymExpr, len: usize) -> Self {
        Self::from_kind(cx, SymBytesKind::Slice { bytes, offset, len })
    }

    pub(crate) fn sized(cx: &mut SymCx, bytes: Self, size: SymExpr, max_size: usize) -> Self {
        if max_size == 0 {
            return Self::empty(cx);
        }
        if let Some(size) = size.eval() {
            let size = usize::try_from(size).map_or(max_size, |size| size.min(max_size));
            let bytes = bytes.slice_concrete(cx, 0, size);
            let padding = Self::concrete(cx, vec![0; max_size - size]);
            return Self::concat(cx, [bytes, padding]);
        }
        Self::from_kind(cx, SymBytesKind::Sized { bytes, size, max_size })
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

    pub(crate) fn byte(&self, cx: &mut SymCx, offset: usize) -> SymExpr {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => bytes
                .get(offset)
                .copied()
                .map(|byte| SymExpr::constant(cx, U256::from(byte)))
                .unwrap_or_else(|| SymExpr::zero(cx)),
            SymBytesKind::Exprs(bytes) => {
                bytes.get(offset).cloned().unwrap_or_else(|| SymExpr::zero(cx))
            }
            SymBytesKind::Word(word) => {
                if offset >= 32 {
                    SymExpr::zero(cx)
                } else if let Some(byte) = word.known_byte(offset) {
                    SymExpr::constant(cx, U256::from(byte))
                } else {
                    word.extracted_byte(cx, offset)
                }
            }
            SymBytesKind::Concat(values) => {
                let mut offset = offset;
                for bytes in values {
                    if offset < bytes.len() {
                        return bytes.byte(cx, offset);
                    }
                    offset = offset.saturating_sub(bytes.len());
                }
                SymExpr::zero(cx)
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len } => {
                if offset >= *len {
                    SymExpr::zero(cx)
                } else if let Some(base_offset) = base_offset.eval() {
                    let Ok(base_offset) = usize::try_from(base_offset) else {
                        return SymExpr::zero(cx);
                    };
                    let Some(offset) = base_offset.checked_add(offset) else {
                        return SymExpr::zero(cx);
                    };
                    bytes.byte(cx, offset)
                } else {
                    bytes.byte_dynamic_with_delta(cx, base_offset, offset)
                }
            }
            SymBytesKind::Sized { bytes, size, max_size } => {
                if offset >= *max_size {
                    return SymExpr::zero(cx);
                }
                let source = bytes.byte(cx, offset);
                let offset = SymExpr::constant(cx, U256::from(offset));
                let condition = SymBoolExpr::cmp(cx, SymCmpOp::Ult, offset, size.clone());
                let zero = SymExpr::zero(cx);
                SymExpr::ite(cx, condition, source, zero)
            }
        }
    }

    pub(crate) fn byte_dynamic_with_delta(
        &self,
        cx: &mut SymCx,
        offset: &SymExpr,
        delta: usize,
    ) -> SymExpr {
        let mut result = SymExpr::zero(cx);
        for candidate in (delta..self.len()).rev() {
            let candidate_offset = candidate - delta;
            let candidate_expr = SymExpr::constant(cx, U256::from(candidate_offset));
            let condition = SymBoolExpr::eq(cx, offset.clone(), candidate_expr);
            let byte = self.byte(cx, candidate);
            result = SymExpr::ite(cx, condition, byte, result);
        }
        result
    }

    pub(crate) fn slice_concrete(&self, cx: &mut SymCx, offset: usize, len: usize) -> Self {
        if len == 0 {
            return Self::empty(cx);
        }
        if offset == 0 && len == self.len() {
            return self.clone();
        }
        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let out = if offset.checked_add(len).is_some_and(|end| end <= bytes.len()) {
                    bytes[offset..offset + len].to_vec()
                } else {
                    (0..len)
                        .map(|idx| bytes.get(offset + idx).copied().unwrap_or_default())
                        .collect()
                };
                Self::concrete(cx, out)
            }
            SymBytesKind::Exprs(bytes) => {
                let bytes = (0..len)
                    .map(|idx| {
                        bytes.get(offset + idx).cloned().unwrap_or_else(|| SymExpr::zero(cx))
                    })
                    .collect();
                Self::exprs(cx, bytes)
            }
            SymBytesKind::Word(word) if offset == 0 && len == 32 => Self::word(cx, word.clone()),
            SymBytesKind::Word(_) | SymBytesKind::Sized { .. } => {
                let offset = SymExpr::constant(cx, U256::from(offset));
                Self::slice_node(cx, self.clone(), offset, len)
            }
            SymBytesKind::Slice { bytes, offset: base_offset, .. } => {
                if let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(offset) = base_offset.checked_add(offset)
                {
                    bytes.slice_concrete(cx, offset, len)
                } else {
                    let offset = SymExpr::constant(cx, U256::from(offset));
                    Self::slice_node(cx, self.clone(), offset, len)
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
                        out.push(bytes.slice_concrete(cx, offset, take));
                    }
                    len -= take;
                    offset = 0;
                }

                if len != 0 {
                    out.push(Self::concrete(cx, vec![0; len]));
                }
                Self::concat(cx, out)
            }
        }
    }

    pub(crate) fn read_offset(&self, cx: &mut SymCx, offset: SymExpr, len: usize) -> Self {
        Self::slice(cx, self.clone(), offset, len)
    }

    pub(crate) fn word_at(&self, cx: &mut SymCx, offset: usize) -> SymExpr {
        if let Some(word) = self.word_fragment_at(cx, offset, 32, 0) {
            return word;
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let mut word = [0u8; 32];
                if offset < bytes.len() {
                    let take = (bytes.len() - offset).min(32);
                    word[..take].copy_from_slice(&bytes[offset..offset + take]);
                }
                SymExpr::constant(cx, U256::from_be_bytes(word))
            }
            SymBytesKind::Exprs(bytes) => {
                let bytes = (0..32)
                    .map(|idx| {
                        bytes.get(offset + idx).cloned().unwrap_or_else(|| SymExpr::zero(cx))
                    })
                    .collect::<Vec<_>>();
                SymExpr::from_bytes(cx, bytes)
            }
            SymBytesKind::Word(word) if offset == 0 => word.clone(),
            SymBytesKind::Word(_) if offset >= 32 => SymExpr::zero(cx),
            SymBytesKind::Slice { bytes, offset: base_offset, len } => {
                if offset.checked_add(32).is_some_and(|end| end <= *len)
                    && let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(base_offset) = base_offset.checked_add(offset)
                {
                    return bytes.word_at(cx, base_offset);
                }
                let bytes = (0..32).map(|idx| self.byte(cx, offset + idx)).collect::<Vec<_>>();
                SymExpr::from_bytes(cx, bytes)
            }
            _ => {
                let bytes = (0..32).map(|idx| self.byte(cx, offset + idx)).collect::<Vec<_>>();
                SymExpr::from_bytes(cx, bytes)
            }
        }
    }

    pub(crate) fn right_aligned_word(&self, cx: &mut SymCx, offset: usize, len: usize) -> SymExpr {
        debug_assert!(len <= 32);
        let len = len.min(32);
        if let Some(word) = self.word_fragment_at(cx, offset, len, 32 - len) {
            return word;
        }

        let mut bytes = Vec::with_capacity(32);
        for _ in 0..32 - len {
            bytes.push(SymExpr::zero(cx));
        }
        bytes.extend((0..len).map(|idx| self.byte(cx, offset + idx)));
        SymExpr::from_bytes(cx, bytes)
    }

    fn word_fragment_at(
        &self,
        cx: &mut SymCx,
        offset: usize,
        len: usize,
        out_offset: usize,
    ) -> Option<SymExpr> {
        debug_assert!(len <= 32);
        debug_assert!(out_offset <= 32);
        debug_assert!(out_offset + len <= 32);

        if len == 0 {
            return Some(SymExpr::zero(cx));
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                let mut word = [0u8; 32];
                if offset < bytes.len() {
                    let take = (bytes.len() - offset).min(len);
                    word[out_offset..out_offset + take]
                        .copy_from_slice(&bytes[offset..offset + take]);
                }
                Some(SymExpr::constant(cx, U256::from_be_bytes(word)))
            }
            SymBytesKind::Word(word) => {
                Self::word_expr_fragment(cx, word.clone(), offset, len, out_offset)
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len: slice_len } => {
                let available = slice_len.saturating_sub(offset).min(len);
                if available == 0 {
                    return Some(SymExpr::zero(cx));
                }
                let base_offset =
                    base_offset.eval().and_then(|offset| usize::try_from(offset).ok())?;
                bytes.word_fragment_at(cx, base_offset.checked_add(offset)?, available, out_offset)
            }
            SymBytesKind::Concat(values) => {
                let mut offset = offset;
                let mut out_offset = out_offset;
                let mut remaining = len;
                let mut out = SymExpr::zero(cx);

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
                    let fragment = bytes.word_fragment_at(cx, offset, take, out_offset)?;
                    out = SymExpr::binop(cx, SymBinOp::Or, out, fragment);
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
        cx: &mut SymCx,
        word: SymExpr,
        offset: usize,
        len: usize,
        out_offset: usize,
    ) -> Option<SymExpr> {
        let len = 32usize.checked_sub(offset)?.min(len);
        if len == 0 {
            return Some(SymExpr::zero(cx));
        }
        if offset == 0 && len == 32 && out_offset == 0 {
            return Some(word);
        }

        let src_trailing_bits = (32 - (offset + len)) * 8;
        let dst_trailing_bits = (32 - (out_offset + len)) * 8;
        let mask = mask_bits(U256::MAX, len * 8);

        let src_trailing_bits = SymExpr::constant(cx, U256::from(src_trailing_bits));
        let expr = SymExpr::binop(cx, SymBinOp::Shr, word, src_trailing_bits);
        let mask = SymExpr::constant(cx, mask);
        let expr = SymExpr::binop(cx, SymBinOp::And, expr, mask);
        let dst_trailing_bits = SymExpr::constant(cx, U256::from(dst_trailing_bits));
        Some(SymExpr::binop(cx, SymBinOp::Shl, expr, dst_trailing_bits))
    }

    pub(crate) fn materialize(&self, cx: &mut SymCx) -> Vec<SymExpr> {
        let mut out = Vec::with_capacity(self.len());
        self.append_expr_range(cx, 0, self.len(), &mut out);
        out
    }

    pub(crate) fn same_bytes(&self, cx: &mut SymCx, other: &Self) -> bool {
        self.len() == other.len()
            && (0..self.len()).all(|idx| self.byte(cx, idx) == other.byte(cx, idx))
    }

    pub(crate) fn prefix_condition(&self, cx: &mut SymCx, prefix: &Self) -> Option<SymBoolExpr> {
        if prefix.len() > self.len() {
            return None;
        }
        let mut conditions = Vec::new();
        for idx in 0..prefix.len() {
            let actual = self.byte(cx, idx);
            let expected = prefix.byte(cx, idx);
            if actual == expected {
                continue;
            }
            match (actual.as_const(), expected.as_const()) {
                (Some(actual), Some(expected)) if actual.to::<u8>() == expected.to::<u8>() => {}
                (Some(_), Some(_)) => return None,
                _ => conditions.push(SymBoolExpr::eq(cx, actual, expected)),
            }
        }
        Some(SymBoolExpr::and(cx, conditions))
    }

    pub(crate) fn eval_model<M: SymbolicModelLookup + ?Sized>(
        &self,
        cx: &mut SymCx,
        model: &M,
    ) -> Result<Vec<u8>, SymbolicError> {
        match self.kind() {
            SymBytesKind::Concrete(bytes) => Ok(bytes.clone()),
            _ => (0..self.len())
                .map(|idx| Ok(self.byte(cx, idx).eval_model(model)?.to::<u8>()))
                .collect(),
        }
    }

    pub(crate) fn concrete_bytes(
        &self,
        cx: &mut SymCx,
        reason: &'static str,
    ) -> Result<Vec<u8>, SymbolicError> {
        let mut out = Vec::with_capacity(self.len());
        self.append_concrete_range(cx, 0, self.len(), &mut out, reason)?;
        Ok(out)
    }

    fn append_concrete_range(
        &self,
        cx: &mut SymCx,
        mut offset: usize,
        mut len: usize,
        out: &mut Vec<u8>,
        reason: &'static str,
    ) -> Result<(), SymbolicError> {
        if len == 0 {
            return Ok(());
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                if offset >= bytes.len() {
                    out.resize(out.len() + len, 0);
                    return Ok(());
                }

                let take = (bytes.len() - offset).min(len);
                out.extend_from_slice(&bytes[offset..offset + take]);
                if len > take {
                    out.resize(out.len() + len - take, 0);
                }
                Ok(())
            }
            SymBytesKind::Exprs(bytes) => {
                for idx in 0..len {
                    let Some(byte) = offset.checked_add(idx).and_then(|idx| bytes.get(idx)) else {
                        out.push(0);
                        continue;
                    };
                    let Some(byte) = byte.as_const() else {
                        return Err(SymbolicError::Unsupported(reason));
                    };
                    out.push(byte.to::<u8>());
                }
                Ok(())
            }
            SymBytesKind::Concat(values) => {
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
                    bytes.append_concrete_range(cx, offset, take, out, reason)?;
                    len -= take;
                    offset = 0;
                }

                if len != 0 {
                    out.resize(out.len() + len, 0);
                }
                Ok(())
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len: slice_len } => {
                if offset >= *slice_len {
                    out.resize(out.len() + len, 0);
                    return Ok(());
                }

                let take = (*slice_len - offset).min(len);
                if let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(offset) = base_offset.checked_add(offset)
                {
                    bytes.append_concrete_range(cx, offset, take, out, reason)?;
                    if len > take {
                        out.resize(out.len() + len - take, 0);
                    }
                } else {
                    let bytes = self.slice_concrete(cx, offset, len).materialize(cx);
                    out.extend(concrete_expr_bytes(&bytes, reason)?);
                }
                Ok(())
            }
            _ => {
                let bytes = self.slice_concrete(cx, offset, len).materialize(cx);
                out.extend(concrete_expr_bytes(&bytes, reason)?);
                Ok(())
            }
        }
    }

    fn append_expr_range(
        &self,
        cx: &mut SymCx,
        mut offset: usize,
        mut len: usize,
        out: &mut Vec<SymExpr>,
    ) {
        if len == 0 {
            return;
        }

        match self.kind() {
            SymBytesKind::Concrete(bytes) => {
                if offset >= bytes.len() {
                    append_zero_exprs(cx, out, len);
                    return;
                }

                let take = (bytes.len() - offset).min(len);
                out.extend(
                    bytes[offset..offset + take]
                        .iter()
                        .copied()
                        .map(|byte| SymExpr::constant(cx, U256::from(byte))),
                );
                if len > take {
                    append_zero_exprs(cx, out, len - take);
                }
            }
            SymBytesKind::Exprs(bytes) => {
                if offset >= bytes.len() {
                    append_zero_exprs(cx, out, len);
                    return;
                }

                let take = (bytes.len() - offset).min(len);
                out.extend(bytes[offset..offset + take].iter().cloned());
                if len > take {
                    append_zero_exprs(cx, out, len - take);
                }
            }
            SymBytesKind::Concat(values) => {
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
                    bytes.append_expr_range(cx, offset, take, out);
                    len -= take;
                    offset = 0;
                }

                if len != 0 {
                    append_zero_exprs(cx, out, len);
                }
            }
            SymBytesKind::Slice { bytes, offset: base_offset, len: slice_len } => {
                if offset >= *slice_len {
                    append_zero_exprs(cx, out, len);
                    return;
                }

                let take = (*slice_len - offset).min(len);
                if let Some(base_offset) = base_offset.eval()
                    && let Ok(base_offset) = usize::try_from(base_offset)
                    && let Some(offset) = base_offset.checked_add(offset)
                {
                    bytes.append_expr_range(cx, offset, take, out);
                    if len > take {
                        append_zero_exprs(cx, out, len - take);
                    }
                } else {
                    append_byte_range(cx, self, offset, len, out);
                }
            }
            SymBytesKind::Word(_) | SymBytesKind::Sized { .. } => {
                append_byte_range(cx, self, offset, len, out);
            }
        }
    }
}

fn append_zero_exprs(cx: &mut SymCx, out: &mut Vec<SymExpr>, len: usize) {
    out.extend((0..len).map(|_| SymExpr::zero(cx)));
}

fn append_byte_range(
    cx: &mut SymCx,
    bytes: &SymBytes,
    offset: usize,
    len: usize,
    out: &mut Vec<SymExpr>,
) {
    out.extend((0..len).map(|idx| {
        offset
            .checked_add(idx)
            .map(|offset| bytes.byte(cx, offset))
            .unwrap_or_else(|| SymExpr::zero(cx))
    }));
}
