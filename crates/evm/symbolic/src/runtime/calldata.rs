use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SymCalldata {
    size: usize,
    size_word: SymExpr,
    bytes: SymBytes,
}

impl SymCalldata {
    pub(crate) fn from_bytes(cx: &mut SymCx, bytes: SymBytes) -> Self {
        let size = bytes.len();
        Self { size_word: cx.constant(U256::from(size)), size, bytes }
    }

    pub(crate) fn from_bytes_with_size(bytes: SymBytes, size_word: SymExpr) -> Self {
        Self { size: bytes.len(), size_word, bytes }
    }

    pub(crate) fn size_word(&self) -> SymExpr {
        self.size_word.clone()
    }

    pub(crate) fn load_word(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return Ok(cx.zero());
            };
            self.load(cx, offset)
        } else {
            self.load_dynamic(cx, &offset)
        }
    }

    pub(crate) fn load(&self, cx: &mut SymCx, offset: usize) -> Result<SymExpr, SymbolicError> {
        Ok(self.bytes.word_at(cx, offset))
    }

    pub(crate) fn load_dynamic(
        &self,
        cx: &mut SymCx,
        offset: &SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        let mut result = cx.zero();
        for candidate in (0..self.size).rev() {
            let candidate_expr = cx.constant(U256::from(candidate));
            let condition = cx.eq(offset.clone(), candidate_expr);
            let word = self.load(cx, candidate)?;
            result = cx.ite(condition, word, result);
        }
        Ok(result)
    }

    pub(crate) fn read_bytes_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> SymBytes {
        self.bytes.read_offset(cx, offset, size)
    }
}

impl BoundedCopySize {
    pub(crate) fn read_from_memory(
        &self,
        cx: &mut SymCx,
        memory: &SymMemory,
        offset: SymExpr,
    ) -> SymBytes {
        match self {
            Self::Concrete(size) => memory.read_bytes_offset(cx, offset, *size),
            Self::Symbolic { size, max_size } => {
                memory.read_bytes_symbolic_size(cx, offset, size.clone(), *max_size)
            }
        }
    }

    pub(crate) fn size_word(&self, cx: &mut SymCx) -> SymExpr {
        match self {
            Self::Concrete(size) => cx.constant(U256::from(*size)),
            Self::Symbolic { size, .. } => size.clone(),
        }
    }

    pub(crate) fn parts(&self, cx: &mut SymCx) -> (SymExpr, usize, bool) {
        match self {
            Self::Concrete(size) => (cx.constant(U256::from(*size)), *size, false),
            Self::Symbolic { size, max_size } => (size.clone(), *max_size, true),
        }
    }

    pub(crate) fn calldata(&self, cx: &mut SymCx, input: SymBytes) -> SymCalldata {
        match self {
            Self::Concrete(_) => SymCalldata::from_bytes(cx, input),
            Self::Symbolic { size, .. } => SymCalldata::from_bytes_with_size(input, size.clone()),
        }
    }
}
