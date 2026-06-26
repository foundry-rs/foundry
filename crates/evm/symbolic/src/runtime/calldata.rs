use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SymCalldata {
    size: usize,
    size_word: SymWord,
    bytes: Arc<[SymWord]>,
}

impl SymCalldata {
    pub(crate) fn new(bytes: Vec<SymWord>) -> Self {
        Self::from_shared(bytes.into())
    }

    pub(crate) fn from_shared(bytes: Arc<[SymWord]>) -> Self {
        Self { size_word: SymWord::constant(U256::from(bytes.len())), size: bytes.len(), bytes }
    }

    pub(crate) fn new_symbolic_size(bytes: Vec<SymWord>, size_word: SymWord) -> Self {
        Self { size: bytes.len(), size_word, bytes: bytes.into() }
    }

    pub(crate) fn size_word(&self) -> SymWord {
        self.size_word.clone()
    }

    pub(crate) fn load_word(&self, offset: SymWord) -> Result<SymWord, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return Ok(SymWord::zero());
            }
            self.load(offset.to::<usize>())
        } else {
            self.load_dynamic(offset.as_expr())
        }
    }

    pub(crate) fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    pub(crate) fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    pub(crate) fn load_dynamic(&self, offset: &SymExpr) -> Result<SymWord, SymbolicError> {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (0..self.size).rev() {
            result = SymExpr::ite(
                BoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate))),
                self.load(candidate)?.into_expr(),
                result,
            );
        }
        Ok(SymWord::expr(result))
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymWord {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            result = SymExpr::ite(
                BoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                self.byte(candidate).into_expr(),
                result,
            );
        }
        SymWord::expr(result)
    }
}

pub(crate) fn call_input_from_memory(
    memory: &SymMemory,
    offset: SymWord,
    size: &BoundedCopySize,
) -> Vec<SymWord> {
    match size {
        BoundedCopySize::Concrete(size) => memory.read_bytes_offset(offset, *size),
        BoundedCopySize::Symbolic { size, max_size } => {
            memory.read_bytes_symbolic_size(offset, size.clone(), *max_size)
        }
    }
}

pub(crate) fn bounded_copy_size_word(size: &BoundedCopySize) -> SymWord {
    match size {
        BoundedCopySize::Concrete(size) => SymWord::constant(U256::from(*size)),
        BoundedCopySize::Symbolic { size, .. } => size.clone(),
    }
}

pub(crate) fn bounded_copy_size_parts(size: &BoundedCopySize) -> (SymWord, usize, bool) {
    match size {
        BoundedCopySize::Concrete(size) => (SymWord::constant(U256::from(*size)), *size, false),
        BoundedCopySize::Symbolic { size, max_size } => (size.clone(), *max_size, true),
    }
}

pub(crate) fn calldata_from_call_input(input: Vec<SymWord>, size: &BoundedCopySize) -> SymCalldata {
    match size {
        BoundedCopySize::Concrete(_) => SymCalldata::new(input),
        BoundedCopySize::Symbolic { size, .. } => {
            SymCalldata::new_symbolic_size(input, size.clone())
        }
    }
}
