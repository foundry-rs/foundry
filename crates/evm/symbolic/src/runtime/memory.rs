use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct SymStack(Vec<SymWord>);

impl SymStack {
    pub(crate) fn push(&mut self, value: SymWord) -> Result<(), SymbolicError> {
        if self.0.len() >= EVM_STACK_LIMIT {
            return Err(SymbolicError::StackOverflow);
        }
        self.0.push(value);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Result<SymWord, SymbolicError> {
        self.0.pop().ok_or(SymbolicError::StackUnderflow)
    }

    pub(crate) fn peek(&self, index_from_top: usize) -> Result<&SymWord, SymbolicError> {
        self.0
            .get(
                self.0
                    .len()
                    .checked_sub(index_from_top + 1)
                    .ok_or(SymbolicError::StackUnderflow)?,
            )
            .ok_or(SymbolicError::StackUnderflow)
    }

    pub(crate) fn swap(&mut self, index_from_top: usize) -> Result<(), SymbolicError> {
        let len = self.0.len();
        let other = len.checked_sub(index_from_top + 1).ok_or(SymbolicError::StackUnderflow)?;
        self.0.swap(len - 1, other);
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) enum BoundedCopySize {
    Concrete(usize),
    Symbolic { size: SymWord, max_size: usize },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SymMemory {
    bytes: HashMap<usize, SymWord>,
    byte_epochs: HashMap<usize, u64>,
    symbolic_writes: Vec<SymbolicMemoryWrite>,
    epoch: u64,
    size: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct SymbolicMemoryWrite {
    epoch: u64,
    offset: Expr,
    bytes: Arc<[SymWord]>,
}

pub(crate) fn memory_size_after_access(offset: usize, len: usize) -> usize {
    let Some(end) = offset.checked_add(len) else {
        return usize::MAX & !31usize;
    };
    end.checked_add(31).map(|size| size & !31usize).unwrap_or(usize::MAX & !31usize)
}

pub(crate) fn memory_size_after_symbolic_access(offset: &Expr, len: U256) -> Expr {
    let end = Expr::op(ExprOp::Add, offset.clone(), Expr::constant(len));
    Expr::op(
        ExprOp::And,
        Expr::op(ExprOp::Add, end, Expr::constant(U256::from(31))),
        Expr::constant(!U256::from(31)),
    )
}

pub(crate) fn max_u256_expr(left: Expr, right: Expr) -> Expr {
    if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
        return Expr::constant(left_value.max(right_value));
    }
    if left == right {
        left
    } else {
        Expr::ite(BoolExpr::cmp(BoolExprOp::Ult, left.clone(), right.clone()), right, left)
    }
}

impl SymMemory {
    pub(crate) fn store_word(&mut self, offset: usize, value: SymWord) {
        self.store_bytes(offset, word_bytes(value));
    }

    pub(crate) fn store_word_offset(&mut self, offset: SymWord, value: SymWord) {
        if let Some(offset) = offset.as_const() {
            if offset <= U256::from(usize::MAX) {
                self.store_word(offset.to::<usize>(), value);
            }
        } else {
            self.store_symbolic_bytes(offset.into_expr(), word_bytes(value));
        }
    }

    pub(crate) fn store_byte(&mut self, offset: usize, value: SymWord) {
        self.store_bytes(offset, vec![low_byte(value)]);
    }

    pub(crate) fn store_byte_offset(&mut self, offset: SymWord, value: SymWord) {
        if let Some(offset) = offset.as_const() {
            if offset <= U256::from(usize::MAX) {
                self.store_byte(offset.to::<usize>(), value);
            }
        } else {
            self.store_symbolic_bytes(offset.into_expr(), vec![low_byte(value)]);
        }
    }

    pub(crate) fn store_bytes(&mut self, offset: usize, bytes: Vec<SymWord>) {
        if bytes.is_empty() {
            return;
        }
        self.epoch = self.epoch.saturating_add(1);
        self.size = self.size.max(memory_size_after_access(offset, bytes.len()));
        for (idx, byte) in bytes.into_iter().enumerate() {
            let offset = offset + idx;
            self.bytes.insert(offset, byte);
            self.byte_epochs.insert(offset, self.epoch);
        }
    }

    pub(crate) fn store_symbolic_bytes(&mut self, offset: Expr, bytes: Vec<SymWord>) {
        if bytes.is_empty() {
            return;
        }
        self.epoch = self.epoch.saturating_add(1);
        self.symbolic_writes.push(SymbolicMemoryWrite {
            epoch: self.epoch,
            offset,
            bytes: bytes.into(),
        });
    }

    pub(crate) fn store_bytes_offset(&mut self, offset: SymWord, bytes: Vec<SymWord>) {
        if let Some(offset) = offset.as_const() {
            if offset <= U256::from(usize::MAX) {
                self.store_bytes(offset.to::<usize>(), bytes);
            }
        } else {
            self.store_symbolic_bytes(offset.into_expr(), bytes);
        }
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    pub(crate) fn load_word_offset(&self, offset: SymWord) -> Result<SymWord, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return Ok(SymWord::zero());
            }
            self.load_word(offset.to::<usize>())
        } else {
            Ok(word_from_bytes(self.read_bytes_offset(offset, 32)))
        }
    }

    pub(crate) fn read_concrete(
        &self,
        offset: usize,
        size: usize,
    ) -> Result<Vec<u8>, SymbolicError> {
        let mut out = vec![0u8; size];
        for (idx, byte) in out.iter_mut().enumerate() {
            if let Some(value) = self.byte(offset + idx).as_const() {
                *byte = value.to::<u8>();
            } else {
                return Err(SymbolicError::Unsupported("symbolic memory read"));
            }
        }
        Ok(out)
    }

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymWord> {
        (0..size).map(|idx| self.byte(offset + idx)).collect()
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return vec![SymWord::zero(); size];
            }
            self.read_bytes(offset.to::<usize>(), size)
        } else {
            let offset = offset.into_expr();
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn read_bytes_symbolic_size(
        &self,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Vec<SymWord> {
        let size = size.into_expr();
        let zero = Expr::constant(U256::ZERO);
        self.read_bytes_offset(offset, max_size)
            .into_iter()
            .enumerate()
            .map(|(idx, source)| {
                SymWord::expr(Expr::ite(
                    BoolExpr::cmp(BoolExprOp::Ult, Expr::constant(U256::from(idx)), size.clone()),
                    source.into_expr(),
                    zero.clone(),
                ))
            })
            .collect()
    }

    pub(crate) fn byte(&self, offset: usize) -> SymWord {
        let (base, base_epoch) = self.base_byte(offset);
        let mut result = base.clone_expr();
        let mut has_symbolic_match = false;
        for write in self.symbolic_writes.iter().filter(|write| write.epoch > base_epoch) {
            for (idx, byte) in write.bytes.iter().enumerate() {
                has_symbolic_match = true;
                result = Expr::ite(
                    BoolExpr::eq(
                        Expr::add_const(write.offset.clone(), U256::from(idx)),
                        Expr::constant(U256::from(offset)),
                    ),
                    byte.clone_expr(),
                    result,
                );
            }
        }
        if has_symbolic_match { SymWord::expr(result) } else { base }
    }

    pub(crate) fn base_byte(&self, offset: usize) -> (SymWord, u64) {
        (
            self.bytes.get(&offset).cloned().unwrap_or_else(SymWord::zero),
            self.byte_epochs.get(&offset).copied().unwrap_or_default(),
        )
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &Expr, delta: usize) -> SymWord {
        let mut result = Expr::constant(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            let (byte, base_epoch) = self.base_byte(candidate);
            let mut candidate_result = byte.into_expr();
            for write in self.symbolic_writes.iter().filter(|write| write.epoch > base_epoch) {
                for (idx, byte) in write.bytes.iter().enumerate() {
                    candidate_result = Expr::ite(
                        BoolExpr::eq(
                            Expr::add_const(write.offset.clone(), U256::from(idx)),
                            Expr::constant(U256::from(candidate)),
                        ),
                        byte.clone_expr(),
                        candidate_result,
                    );
                }
            }
            result = Expr::ite(
                BoolExpr::eq(offset.clone(), Expr::constant(U256::from(candidate - delta))),
                candidate_result,
                result,
            );
        }
        SymWord::expr(result)
    }

    pub(crate) fn size_word(&self) -> SymWord {
        let mut size = Expr::constant(U256::from(self.size));
        for write in &self.symbolic_writes {
            let write_size =
                memory_size_after_symbolic_access(&write.offset, U256::from(write.bytes.len()));
            size = max_u256_expr(size, write_size);
        }
        SymWord::expr(size)
    }

    #[cfg(test)]
    pub(crate) fn copy_symbolic(&mut self, dest: usize, src: Vec<SymWord>) {
        self.store_bytes(dest, src);
    }

    pub(crate) fn copy_symbolic_offset(&mut self, dest: SymWord, src: Vec<SymWord>) {
        self.store_bytes_offset(dest, src);
    }

    #[cfg(test)]
    pub(crate) fn copy_symbolic_size(&mut self, dest: usize, size: SymWord, src: Vec<SymWord>) {
        self.copy_symbolic_size_offset(SymWord::constant(U256::from(dest)), size, src)
            .expect("concrete symbolic-size memory copy cannot fail");
    }

    pub(crate) fn copy_symbolic_size_offset(
        &mut self,
        dest: SymWord,
        size: SymWord,
        src: Vec<SymWord>,
    ) -> Result<(), SymbolicError> {
        if src.is_empty() {
            return Ok(());
        }
        let size = size.into_expr();
        if let Some(dest) = dest.as_const() {
            if dest <= U256::from(usize::MAX) {
                let dest = dest.to::<usize>();
                let bytes = src
                    .into_iter()
                    .enumerate()
                    .map(|(idx, source)| {
                        self.symbolic_copy_size_byte(dest + idx, idx, &size, source)
                    })
                    .collect();
                self.store_bytes(dest, bytes);
            }
        } else {
            let dest = dest.into_expr();
            let bytes = src
                .into_iter()
                .enumerate()
                .map(|(idx, source)| {
                    let existing = self.byte_dynamic_with_delta(&dest, idx);
                    symbolic_copy_size_byte(idx, &size, source, existing)
                })
                .collect();
            self.store_symbolic_bytes(dest, bytes);
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn copy_calldata(
        &mut self,
        dest: usize,
        offset: usize,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        self.store_bytes(dest, (0..size).map(|idx| calldata.byte(offset + idx)).collect());
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn copy_calldata_offset(
        &mut self,
        dest: usize,
        offset: SymWord,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        self.copy_calldata_to_offset(SymWord::constant(U256::from(dest)), offset, size, calldata)
    }

    pub(crate) fn copy_calldata_to_offset(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                self.copy_symbolic_offset(dest, vec![SymWord::zero(); size]);
            } else {
                self.store_bytes_offset(
                    dest,
                    (0..size).map(|idx| calldata.byte(offset.to::<usize>() + idx)).collect(),
                );
            }
        } else {
            let offset = offset.into_expr();
            self.store_bytes_offset(
                dest,
                (0..size).map(|idx| calldata.byte_dynamic_with_delta(&offset, idx)).collect(),
            );
        }
        Ok(())
    }

    pub(crate) fn copy_calldata_symbolic_size(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        let bytes = if let Some(offset) = offset.as_const() {
            let offset =
                if offset > U256::from(usize::MAX) { None } else { Some(offset.to::<usize>()) };
            (0..max_size)
                .map(|idx| {
                    offset.map(|offset| calldata.byte(offset + idx)).unwrap_or_else(SymWord::zero)
                })
                .collect()
        } else {
            let offset = offset.into_expr();
            (0..max_size).map(|idx| calldata.byte_dynamic_with_delta(&offset, idx)).collect()
        };
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    pub(crate) fn symbolic_copy_size_byte(
        &self,
        dest: usize,
        idx: usize,
        size: &Expr,
        source: SymWord,
    ) -> SymWord {
        let existing = self.byte(dest);
        symbolic_copy_size_byte(idx, size, source, existing)
    }

    pub(crate) fn copy_return_data_to_offset(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
            if offset.to::<usize>().saturating_add(size) > return_data.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        self.store_bytes_offset(dest, return_data.read_bytes_offset(offset, size));
        Ok(())
    }

    pub(crate) fn copy_return_data_symbolic_size(
        &mut self,
        dest: SymWord,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
            if offset.to::<usize>().saturating_add(max_size) > return_data.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        let bytes = return_data.read_bytes_offset(offset, max_size);
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    pub(crate) fn copy_call_output_offset(
        &mut self,
        dest: SymWord,
        size: &BoundedCopySize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        match size {
            BoundedCopySize::Concrete(size) => {
                let size = (*size).min(return_data.len());
                if size != 0 {
                    if return_data.has_symbolic_len() {
                        let bytes = (0..size)
                            .map(|idx| self.call_output_byte(&dest, idx, None, return_data))
                            .collect();
                        self.store_bytes_offset(dest, bytes);
                    } else {
                        self.store_bytes_offset(
                            dest,
                            (0..size).map(|idx| return_data.byte(idx)).collect(),
                        );
                    }
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => {
                let output_size = size.clone_expr();
                let max_size = (*max_size).min(return_data.len());
                if max_size != 0 {
                    let bytes = (0..max_size)
                        .map(|idx| {
                            self.call_output_byte(&dest, idx, Some(&output_size), return_data)
                        })
                        .collect();
                    self.store_bytes_offset(dest, bytes);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn call_output_byte(
        &self,
        dest: &SymWord,
        idx: usize,
        output_size: Option<&Expr>,
        return_data: &SymReturnData,
    ) -> SymWord {
        let mut guards = Vec::new();
        if let Some(output_size) = output_size {
            guards.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                Expr::constant(U256::from(idx)),
                output_size.clone(),
            ));
        }
        if return_data.has_symbolic_len() {
            guards.push(BoolExpr::cmp(
                BoolExprOp::Ult,
                Expr::constant(U256::from(idx)),
                return_data.len_expr(),
            ));
        }
        let guard = BoolExpr::and(guards);
        match guard {
            BoolExpr::Const(true) => return_data.byte(idx),
            BoolExpr::Const(false) => self.call_output_existing_byte(dest, idx),
            guard => SymWord::expr(Expr::ite(
                guard,
                return_data.byte(idx).into_expr(),
                self.call_output_existing_byte(dest, idx).into_expr(),
            )),
        }
    }

    pub(crate) fn call_output_existing_byte(&self, dest: &SymWord, idx: usize) -> SymWord {
        if let Some(dest) = dest.as_const() {
            if dest <= U256::from(usize::MAX) {
                self.byte(dest.to::<usize>() + idx)
            } else {
                SymWord::zero()
            }
        } else {
            self.byte_dynamic_with_delta(dest.as_expr(), idx)
        }
    }

    #[cfg(test)]
    pub(crate) fn copy_memory_offset(
        &mut self,
        dest: usize,
        src: SymWord,
        size: usize,
    ) -> Result<(), SymbolicError> {
        self.copy_memory_to_offset(SymWord::constant(U256::from(dest)), src, size)
    }

    pub(crate) fn copy_memory_to_offset(
        &mut self,
        dest: SymWord,
        src: SymWord,
        size: usize,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        let bytes = self.read_bytes_offset(src, size);
        self.store_bytes_offset(dest, bytes);
        Ok(())
    }

    pub(crate) fn copy_memory_symbolic_size(
        &mut self,
        dest: SymWord,
        src: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        let source = self.read_bytes_offset(src, max_size);
        self.copy_symbolic_size_offset(dest, size, source)
    }

    pub(crate) fn return_data(
        &self,
        offset: SymWord,
        size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes(self.read_bytes_offset(offset, size)))
    }

    pub(crate) fn return_data_symbolic_size(
        &self,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes_with_len(
            self.read_bytes_symbolic_size(offset, size.clone(), max_size),
            size,
        ))
    }
}

pub(crate) fn symbolic_copy_size_byte(
    idx: usize,
    size: &Expr,
    source: SymWord,
    existing: SymWord,
) -> SymWord {
    SymWord::expr(Expr::ite(
        BoolExpr::cmp(BoolExprOp::Ult, Expr::constant(U256::from(idx)), size.clone()),
        source.into_expr(),
        existing.into_expr(),
    ))
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymCode {
    bytes: Arc<[SymWord]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuardedOpcode {
    End,
    Concrete(u8),
    SymbolicSize { condition: BoolExpr, opcode: u8 },
}

impl SymCode {
    /// Converts symbolic bytes into code.
    pub(crate) fn from_symbolic_bytes(bytes: Vec<SymWord>) -> Self {
        Self::from_shared_bytes(bytes.into())
    }

    /// Converts shared symbolic bytes into code.
    pub(crate) const fn from_shared_bytes(bytes: Arc<[SymWord]>) -> Self {
        Self { bytes }
    }

    pub(crate) fn concrete(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes
                .into_iter()
                .map(|byte| SymWord::constant(U256::from(byte)))
                .collect::<Vec<_>>()
                .into(),
        }
    }

    pub(crate) fn from_memory_offset(memory: &SymMemory, offset: SymWord, size: usize) -> Self {
        Self::from_symbolic_bytes(memory.read_bytes_offset(offset, size))
    }

    pub(crate) fn from_memory_symbolic_size(
        memory: &SymMemory,
        offset: SymWord,
        size: SymWord,
        max_size: usize,
    ) -> Self {
        Self::from_symbolic_bytes(memory.read_bytes_symbolic_size(offset, size, max_size))
    }

    /// Returns the symbolic code bytes.
    pub(crate) fn bytes(&self) -> &[SymWord] {
        &self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns whether `is_empty` holds.
    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) fn opcode(&self, pc: usize) -> Result<Option<u8>, SymbolicError> {
        self.bytes
            .get(pc)
            .map(|byte| match byte.as_const() {
                Some(value) => Ok(value.to::<u8>()),
                None => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
            })
            .transpose()
    }

    pub(crate) fn guarded_opcode(&self, pc: usize) -> Result<GuardedOpcode, SymbolicError> {
        match self.bytes.get(pc) {
            None => Ok(GuardedOpcode::End),
            Some(byte) if byte.as_const().is_some() => {
                Ok(GuardedOpcode::Concrete(byte.as_const().expect("checked concrete").to::<u8>()))
            }
            Some(byte) => {
                if let ExprInner::Ite(condition, then_expr, else_expr) = byte.as_expr().as_inner()
                    && else_expr.as_const().is_some_and(|value| value.is_zero())
                {
                    match then_expr.as_const() {
                        Some(value) if value.is_zero() => Ok(GuardedOpcode::Concrete(0)),
                        Some(value) => Ok(GuardedOpcode::SymbolicSize {
                            condition: condition.as_ref().clone(),
                            opcode: value.to::<u8>(),
                        }),
                        None => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
                    }
                } else {
                    Err(SymbolicError::Unsupported("symbolic bytecode opcode"))
                }
            }
        }
    }

    pub(crate) fn analysis_opcode(&self, pc: usize) -> Option<u8> {
        self.bytes
            .get(pc)
            .map(|byte| byte.as_const().map_or(opcode::STOP, |value| value.to::<u8>()))
    }

    pub(crate) fn concrete_range(
        &self,
        offset: usize,
        size: usize,
        reason: &'static str,
    ) -> Result<Vec<u8>, SymbolicError> {
        let mut out = Vec::with_capacity(size);
        for idx in 0..size {
            match self.bytes.get(offset + idx) {
                Some(byte) => match byte.as_const() {
                    Some(value) => out.push(value.to::<u8>()),
                    None => return Err(SymbolicError::Unsupported(reason)),
                },
                None => out.push(0),
            }
        }
        Ok(out)
    }

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymWord> {
        (0..size)
            .map(|idx| self.bytes.get(offset + idx).cloned().unwrap_or_else(SymWord::zero))
            .collect()
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return vec![SymWord::zero(); size];
            }
            self.read_bytes(offset.to::<usize>(), size)
        } else {
            let offset = offset.into_expr();
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &Expr, delta: usize) -> SymWord {
        let mut result = Expr::constant(U256::ZERO);
        for candidate in (delta..self.len()).rev() {
            result = Expr::ite(
                BoolExpr::eq(offset.clone(), Expr::constant(U256::from(candidate - delta))),
                self.bytes[candidate].clone_expr(),
                result,
            );
        }
        SymWord::expr(result)
    }

    pub(crate) fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        self.concrete_range(0, self.len(), reason)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SymReturnData {
    len_word: SymWord,
    bytes: Arc<[SymWord]>,
}

impl Default for SymReturnData {
    fn default() -> Self {
        Self { len_word: SymWord::zero(), bytes: Vec::new().into() }
    }
}

impl SymReturnData {
    pub(crate) fn from_words(words: Vec<SymWord>) -> Self {
        let bytes = words.into_iter().flat_map(word_bytes).collect::<Vec<_>>();
        Self::from_symbolic_bytes(bytes)
    }

    pub(crate) fn from_concrete_bytes(bytes: Vec<u8>) -> Self {
        Self::from_symbolic_bytes(
            bytes.into_iter().map(|byte| SymWord::constant(U256::from(byte))).collect(),
        )
    }

    pub(crate) fn from_symbolic_bytes(bytes: Vec<SymWord>) -> Self {
        let len = bytes.len();
        Self { len_word: SymWord::constant(U256::from(len)), bytes: bytes.into() }
    }

    pub(crate) fn from_symbolic_bytes_with_len(bytes: Vec<SymWord>, len_word: SymWord) -> Self {
        Self { len_word, bytes: bytes.into() }
    }

    pub(crate) fn len_word(&self) -> SymWord {
        self.len_word.clone()
    }

    /// Returns the concrete backing byte length.
    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn len_expr(&self) -> Expr {
        self.len_word.clone_expr()
    }

    /// Returns whether `has_symbolic_len` holds.
    pub(crate) fn has_symbolic_len(&self) -> bool {
        self.len_word.as_const().is_none()
    }

    pub(crate) fn byte(&self, offset: usize) -> SymWord {
        self.bytes.get(offset).cloned().unwrap_or_else(SymWord::zero)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymWord, size: usize) -> Vec<SymWord> {
        if let Some(offset) = offset.as_const() {
            if offset > U256::from(usize::MAX) {
                return vec![SymWord::zero(); size];
            }
            let offset = offset.to::<usize>();
            (0..size).map(|idx| self.byte(offset + idx)).collect()
        } else {
            let offset = offset.into_expr();
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &Expr, delta: usize) -> SymWord {
        let mut result = Expr::constant(U256::ZERO);
        for candidate in (delta..self.len()).rev() {
            result = Expr::ite(
                BoolExpr::eq(offset.clone(), Expr::constant(U256::from(candidate - delta))),
                self.bytes[candidate].clone_expr(),
                result,
            );
        }
        SymWord::expr(result)
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymWord, SymbolicError> {
        if offset.saturating_add(32) > self.len() {
            return Err(SymbolicError::Unsupported("out-of-bounds symbolic returndata word"));
        }
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    pub(crate) fn read_concrete(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        let mut out = Vec::with_capacity(self.len());
        for byte in self.bytes.iter() {
            if let Some(value) = byte.as_const() {
                out.push(value.to::<u8>());
            } else {
                return Err(SymbolicError::Unsupported(reason));
            }
        }
        Ok(out)
    }

    pub(crate) fn to_code(&self) -> Result<SymCode, SymbolicError> {
        if self.has_symbolic_len() {
            return Err(SymbolicError::Unsupported(
                "CREATE with symbolic runtime size not modeled",
            ));
        }
        Ok(SymCode::from_shared_bytes(self.bytes.clone()))
    }
}
