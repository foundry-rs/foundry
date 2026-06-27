use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SymCalldata {
    size: usize,
    size_word: SymExpr,
    bytes: SymBytes,
}

impl SymCalldata {
    pub(crate) fn from_bytes(bytes: SymBytes) -> Self {
        let size = bytes.len();
        Self { size_word: SymExpr::constant(U256::from(size)), size, bytes }
    }

    pub(crate) fn from_bytes_with_size(bytes: SymBytes, size_word: SymExpr) -> Self {
        Self { size: bytes.len(), size_word, bytes }
    }

    pub(crate) fn size_word(&self) -> SymExpr {
        self.size_word.clone()
    }

    pub(crate) fn load_word(&self, offset: SymExpr) -> Result<SymExpr, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return Ok(SymExpr::zero());
            };
            self.load(offset)
        } else {
            self.load_dynamic(&offset)
        }
    }

    pub(crate) fn load(&self, offset: usize) -> Result<SymExpr, SymbolicError> {
        Ok(self.bytes.word_at(offset))
    }

    pub(crate) fn load_dynamic(&self, offset: &SymExpr) -> Result<SymExpr, SymbolicError> {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (0..self.size).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate))),
                self.load(candidate)?,
                result,
            );
        }
        Ok(result)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> SymBytes {
        self.bytes.read_offset(offset, size)
    }
}

impl BoundedCopySize {
    pub(crate) fn read_from_memory(&self, memory: &SymMemory, offset: SymExpr) -> SymBytes {
        match self {
            Self::Concrete(size) => memory.read_bytes_offset(offset, *size),
            Self::Symbolic { size, max_size } => {
                memory.read_bytes_symbolic_size(offset, size.clone(), *max_size)
            }
        }
    }

    pub(crate) fn size_word(&self) -> SymExpr {
        match self {
            Self::Concrete(size) => SymExpr::constant(U256::from(*size)),
            Self::Symbolic { size, .. } => size.clone(),
        }
    }

    pub(crate) fn parts(&self) -> (SymExpr, usize, bool) {
        match self {
            Self::Concrete(size) => (SymExpr::constant(U256::from(*size)), *size, false),
            Self::Symbolic { size, max_size } => (size.clone(), *max_size, true),
        }
    }

    pub(crate) fn calldata(&self, input: SymBytes) -> SymCalldata {
        match self {
            Self::Concrete(_) => SymCalldata::from_bytes(input),
            Self::Symbolic { size, .. } => SymCalldata::from_bytes_with_size(input, size.clone()),
        }
    }
}
