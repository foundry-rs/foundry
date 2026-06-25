use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SymCalldata {
    size: usize,
    size_word: SymWord,
    bytes: Vec<SymWord>,
}

impl SymCalldata {
    /// Constructs a new instance.
    pub(crate) fn new(bytes: Vec<SymWord>) -> Self {
        Self { size_word: SymWord::Concrete(U256::from(bytes.len())), size: bytes.len(), bytes }
    }

    /// Implements the `new_symbolic_size` symbolic calldata helper.
    pub(crate) const fn new_symbolic_size(bytes: Vec<SymWord>, size_word: SymWord) -> Self {
        Self { size: bytes.len(), size_word, bytes }
    }

    /// Returns the symbolic calldata size word.
    pub(crate) fn size_word(&self) -> SymWord {
        self.size_word.clone()
    }

    /// Returns the `load_word` symbolic calldata helper result.
    pub(crate) fn load_word(&self, offset: SymWord) -> Result<SymWord, SymbolicError> {
        match offset {
            SymWord::Concrete(offset) => {
                if offset > U256::from(usize::MAX) {
                    return Ok(SymWord::zero());
                }
                self.load(offset.to::<usize>())
            }
            SymWord::Expr(offset) => self.load_dynamic(offset),
        }
    }

    /// Implements the `load` symbolic calldata helper.
    pub(crate) fn load(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    /// Implements the `byte` symbolic calldata helper.
    pub(crate) fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    /// Returns the `load_dynamic` symbolic calldata helper result.
    pub(crate) fn load_dynamic(&self, offset: Expr) -> Result<SymWord, SymbolicError> {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (0..self.size).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate)))),
                Box::new(self.load(candidate)?.into_expr()),
                Box::new(result),
            );
        }
        Ok(SymWord::from_expr(result))
    }

    /// Returns the `byte_dynamic_with_delta` symbolic calldata helper result.
    pub(crate) fn byte_dynamic_with_delta(&self, offset: Expr, delta: usize) -> SymWord {
        let mut result = Expr::Const(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            result = Expr::Ite(
                Box::new(BoolExpr::eq(offset.clone(), Expr::Const(U256::from(candidate - delta)))),
                Box::new(self.byte(candidate).into_expr()),
                Box::new(result),
            );
        }
        SymWord::from_expr(result)
    }
}

/// Implements the `call_input_from_memory` symbolic calldata helper.
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

/// Implements the `bounded_copy_size_word` symbolic calldata helper.
pub(crate) fn bounded_copy_size_word(size: &BoundedCopySize) -> SymWord {
    match size {
        BoundedCopySize::Concrete(size) => SymWord::Concrete(U256::from(*size)),
        BoundedCopySize::Symbolic { size, .. } => size.clone(),
    }
}

/// Implements the `bounded_copy_size_parts` symbolic calldata helper.
pub(crate) fn bounded_copy_size_parts(size: &BoundedCopySize) -> (SymWord, usize, bool) {
    match size {
        BoundedCopySize::Concrete(size) => (SymWord::Concrete(U256::from(*size)), *size, false),
        BoundedCopySize::Symbolic { size, max_size } => (size.clone(), *max_size, true),
    }
}

/// Implements the `calldata_from_call_input` symbolic calldata helper.
pub(crate) fn calldata_from_call_input(input: Vec<SymWord>, size: &BoundedCopySize) -> SymCalldata {
    match size {
        BoundedCopySize::Concrete(_) => SymCalldata::new(input),
        BoundedCopySize::Symbolic { size, .. } => {
            SymCalldata::new_symbolic_size(input, size.clone())
        }
    }
}
