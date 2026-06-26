use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SymCalldata {
    size: usize,
    size_word: SymExpr,
    bytes: Arc<[SymExpr]>,
}

impl SymCalldata {
    pub(crate) fn new(bytes: Vec<SymExpr>) -> Self {
        Self::from_shared(bytes.into())
    }

    pub(crate) fn from_shared(bytes: Arc<[SymExpr]>) -> Self {
        Self { size_word: SymExpr::constant(U256::from(bytes.len())), size: bytes.len(), bytes }
    }

    pub(crate) fn new_symbolic_size(bytes: Vec<SymExpr>, size_word: SymExpr) -> Self {
        Self { size: bytes.len(), size_word, bytes: bytes.into() }
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
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    pub(crate) fn byte(&self, offset: usize) -> SymExpr {
        self.bytes.get(offset).cloned().unwrap_or_else(SymExpr::zero)
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

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                self.byte(candidate),
                result,
            );
        }
        result
    }
}

pub(crate) fn call_input_from_memory(
    memory: &SymMemory,
    offset: SymExpr,
    size: &BoundedCopySize,
) -> Vec<SymExpr> {
    match size {
        BoundedCopySize::Concrete(size) => memory.read_bytes_offset(offset, *size),
        BoundedCopySize::Symbolic { size, max_size } => {
            memory.read_bytes_symbolic_size(offset, size.clone(), *max_size)
        }
    }
}

pub(crate) fn bounded_copy_size_word(size: &BoundedCopySize) -> SymExpr {
    match size {
        BoundedCopySize::Concrete(size) => SymExpr::constant(U256::from(*size)),
        BoundedCopySize::Symbolic { size, .. } => size.clone(),
    }
}

pub(crate) fn bounded_copy_size_parts(size: &BoundedCopySize) -> (SymExpr, usize, bool) {
    match size {
        BoundedCopySize::Concrete(size) => (SymExpr::constant(U256::from(*size)), *size, false),
        BoundedCopySize::Symbolic { size, max_size } => (size.clone(), *max_size, true),
    }
}

pub(crate) fn calldata_from_call_input(input: Vec<SymExpr>, size: &BoundedCopySize) -> SymCalldata {
    match size {
        BoundedCopySize::Concrete(_) => SymCalldata::new(input),
        BoundedCopySize::Symbolic { size, .. } => {
            SymCalldata::new_symbolic_size(input, size.clone())
        }
    }
}
