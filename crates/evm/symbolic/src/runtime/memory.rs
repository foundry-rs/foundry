use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct SymStack(Vec<SymExpr>);

impl SymStack {
    pub(crate) fn push(&mut self, value: SymExpr) -> Result<(), SymbolicError> {
        if self.0.len() >= EVM_STACK_LIMIT {
            return Err(SymbolicError::StackOverflow);
        }
        self.0.push(value);
        Ok(())
    }

    pub(crate) fn pop(&mut self) -> Result<SymExpr, SymbolicError> {
        self.0.pop().ok_or(SymbolicError::StackUnderflow)
    }

    pub(crate) fn peek(&self, index_from_top: usize) -> Result<&SymExpr, SymbolicError> {
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
    Symbolic { size: SymExpr, max_size: usize },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SymMemory {
    bytes: HashMap<usize, SymExpr>,
    byte_epochs: HashMap<usize, u64>,
    symbolic_writes: Vec<SymbolicMemoryWrite>,
    epoch: u64,
    size: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct SymbolicMemoryWrite {
    epoch: u64,
    offset: SymExpr,
    bytes: Arc<[SymExpr]>,
}

impl SymbolicMemoryWrite {
    fn size_after_access(&self) -> SymExpr {
        let end = SymExpr::op(
            SymExprOp::Add,
            self.offset.clone(),
            SymExpr::constant(U256::from(self.bytes.len())),
        );
        SymExpr::op(
            SymExprOp::And,
            SymExpr::op(SymExprOp::Add, end, SymExpr::constant(U256::from(31))),
            SymExpr::constant(!U256::from(31)),
        )
    }
}

impl SymMemory {
    fn size_after_access(offset: usize, len: usize) -> usize {
        let Some(end) = offset.checked_add(len) else {
            return usize::MAX & !31usize;
        };
        end.checked_add(31).map(|size| size & !31usize).unwrap_or(usize::MAX & !31usize)
    }

    fn max_size_word(left: SymExpr, right: SymExpr) -> SymExpr {
        if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
            return SymExpr::constant(left_value.max(right_value));
        }
        if left == right {
            left
        } else {
            SymExpr::ite(
                SymBoolExpr::cmp(SymBoolExprOp::Ult, left.clone(), right.clone()),
                right,
                left,
            )
        }
    }

    pub(crate) fn store_word(&mut self, offset: usize, value: SymExpr) {
        self.store_bytes(offset, word_bytes(value));
    }

    pub(crate) fn store_word_offset(&mut self, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_word(offset, value);
            }
        } else {
            self.store_symbolic_bytes(offset, word_bytes(value));
        }
    }

    pub(crate) fn store_byte(&mut self, offset: usize, value: SymExpr) {
        self.store_bytes(offset, vec![low_byte(value)]);
    }

    pub(crate) fn store_byte_offset(&mut self, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_byte(offset, value);
            }
        } else {
            self.store_symbolic_bytes(offset, vec![low_byte(value)]);
        }
    }

    pub(crate) fn store_bytes(&mut self, offset: usize, bytes: Vec<SymExpr>) {
        if bytes.is_empty() {
            return;
        }
        self.epoch = self.epoch.saturating_add(1);
        self.size = self.size.max(Self::size_after_access(offset, bytes.len()));
        for (idx, byte) in bytes.into_iter().enumerate() {
            let offset = offset + idx;
            self.bytes.insert(offset, byte);
            self.byte_epochs.insert(offset, self.epoch);
        }
    }

    pub(crate) fn store_symbolic_bytes(&mut self, offset: SymExpr, bytes: Vec<SymExpr>) {
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

    pub(crate) fn store_bytes_offset(&mut self, offset: SymExpr, bytes: Vec<SymExpr>) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_bytes(offset, bytes);
            }
        } else {
            self.store_symbolic_bytes(offset, bytes);
        }
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymExpr, SymbolicError> {
        Ok(word_from_bytes((0..32).map(|idx| self.byte(offset + idx))))
    }

    pub(crate) fn load_word_offset(&self, offset: SymExpr) -> Result<SymExpr, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else { return Ok(SymExpr::zero()) };
            self.load_word(offset)
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

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymExpr> {
        (0..size).map(|idx| self.byte(offset + idx)).collect()
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> Vec<SymExpr> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return vec![SymExpr::zero(); size];
            };
            self.read_bytes(offset, size)
        } else {
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn read_bytes_symbolic_size(
        &self,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Vec<SymExpr> {
        let zero = SymExpr::constant(U256::ZERO);
        self.read_bytes_offset(offset, max_size)
            .into_iter()
            .enumerate()
            .map(|(idx, source)| {
                SymExpr::ite(
                    SymBoolExpr::cmp(
                        SymBoolExprOp::Ult,
                        SymExpr::constant(U256::from(idx)),
                        size.clone(),
                    ),
                    source,
                    zero.clone(),
                )
            })
            .collect()
    }

    pub(crate) fn byte(&self, offset: usize) -> SymExpr {
        let (base, base_epoch) = self.base_byte(offset);
        let mut result = base.clone();
        let mut has_symbolic_match = false;
        for write in self.symbolic_writes.iter().filter(|write| write.epoch > base_epoch) {
            for (idx, byte) in write.bytes.iter().enumerate() {
                has_symbolic_match = true;
                result = SymExpr::ite(
                    SymBoolExpr::eq(
                        SymExpr::add_const(write.offset.clone(), U256::from(idx)),
                        SymExpr::constant(U256::from(offset)),
                    ),
                    byte.clone(),
                    result,
                );
            }
        }
        if has_symbolic_match { result } else { base }
    }

    pub(crate) fn base_byte(&self, offset: usize) -> (SymExpr, u64) {
        (
            self.bytes.get(&offset).cloned().unwrap_or_else(SymExpr::zero),
            self.byte_epochs.get(&offset).copied().unwrap_or_default(),
        )
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            let (byte, base_epoch) = self.base_byte(candidate);
            let mut candidate_result = byte;
            for write in self.symbolic_writes.iter().filter(|write| write.epoch > base_epoch) {
                for (idx, byte) in write.bytes.iter().enumerate() {
                    candidate_result = SymExpr::ite(
                        SymBoolExpr::eq(
                            SymExpr::add_const(write.offset.clone(), U256::from(idx)),
                            SymExpr::constant(U256::from(candidate)),
                        ),
                        byte.clone(),
                        candidate_result,
                    );
                }
            }
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                candidate_result,
                result,
            );
        }
        result
    }

    pub(crate) fn size_word(&self) -> SymExpr {
        let mut size = SymExpr::constant(U256::from(self.size));
        for write in &self.symbolic_writes {
            size = Self::max_size_word(size, write.size_after_access());
        }
        size
    }

    #[cfg(test)]
    pub(crate) fn copy_symbolic(&mut self, dest: usize, src: Vec<SymExpr>) {
        self.store_bytes(dest, src);
    }

    pub(crate) fn copy_symbolic_offset(&mut self, dest: SymExpr, src: Vec<SymExpr>) {
        self.store_bytes_offset(dest, src);
    }

    #[cfg(test)]
    pub(crate) fn copy_symbolic_size(&mut self, dest: usize, size: SymExpr, src: Vec<SymExpr>) {
        self.copy_symbolic_size_offset(SymExpr::constant(U256::from(dest)), size, src)
            .expect("concrete symbolic-size memory copy cannot fail");
    }

    pub(crate) fn copy_symbolic_size_offset(
        &mut self,
        dest: SymExpr,
        size: SymExpr,
        src: Vec<SymExpr>,
    ) -> Result<(), SymbolicError> {
        if src.is_empty() {
            return Ok(());
        }
        if let Some(dest) = dest.as_const() {
            if let Ok(dest) = usize::try_from(dest) {
                let bytes = src
                    .into_iter()
                    .enumerate()
                    .map(|(idx, source)| self.copy_size_byte_at(dest + idx, idx, &size, source))
                    .collect();
                self.store_bytes(dest, bytes);
            }
        } else {
            let bytes = src
                .into_iter()
                .enumerate()
                .map(|(idx, source)| {
                    let existing = self.byte_dynamic_with_delta(&dest, idx);
                    Self::copy_size_byte(idx, &size, source, existing)
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
        offset: SymExpr,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        self.copy_calldata_to_offset(SymExpr::constant(U256::from(dest)), offset, size, calldata)
    }

    pub(crate) fn copy_calldata_to_offset(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                self.copy_symbolic_offset(dest, vec![SymExpr::zero(); size]);
                return Ok(());
            };
            self.store_bytes_offset(
                dest,
                (0..size).map(|idx| calldata.byte(offset + idx)).collect(),
            );
        } else {
            self.store_bytes_offset(
                dest,
                (0..size).map(|idx| calldata.byte_dynamic_with_delta(&offset, idx)).collect(),
            );
        }
        Ok(())
    }

    pub(crate) fn copy_calldata_symbolic_size(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        let bytes = if let Some(offset) = offset.as_const() {
            let offset = usize::try_from(offset).ok();
            (0..max_size)
                .map(|idx| {
                    offset.map(|offset| calldata.byte(offset + idx)).unwrap_or_else(SymExpr::zero)
                })
                .collect()
        } else {
            (0..max_size).map(|idx| calldata.byte_dynamic_with_delta(&offset, idx)).collect()
        };
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    fn copy_size_byte_at(
        &self,
        dest: usize,
        idx: usize,
        size: &SymExpr,
        source: SymExpr,
    ) -> SymExpr {
        let existing = self.byte(dest);
        Self::copy_size_byte(idx, size, source, existing)
    }

    fn copy_size_byte(idx: usize, size: &SymExpr, source: SymExpr, existing: SymExpr) -> SymExpr {
        SymExpr::ite(
            SymBoolExpr::cmp(SymBoolExprOp::Ult, SymExpr::constant(U256::from(idx)), size.clone()),
            source,
            existing,
        )
    }

    pub(crate) fn copy_return_data_to_offset(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            };
            if offset.saturating_add(size) > return_data.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        self.store_bytes_offset(dest, return_data.read_bytes_offset(offset, size));
        Ok(())
    }

    pub(crate) fn copy_return_data_symbolic_size(
        &mut self,
        dest: SymExpr,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
        return_data: &SymReturnData,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            };
            if offset.saturating_add(max_size) > return_data.len() {
                return Err(SymbolicError::Unsupported("out-of-bounds symbolic RETURNDATACOPY"));
            }
        }
        let bytes = return_data.read_bytes_offset(offset, max_size);
        self.copy_symbolic_size_offset(dest, size, bytes)
    }

    pub(crate) fn copy_call_output_offset(
        &mut self,
        dest: SymExpr,
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
                let output_size = size.clone();
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
        dest: &SymExpr,
        idx: usize,
        output_size: Option<&SymExpr>,
        return_data: &SymReturnData,
    ) -> SymExpr {
        let mut guards = Vec::new();
        if let Some(output_size) = output_size {
            guards.push(SymBoolExpr::cmp(
                SymBoolExprOp::Ult,
                SymExpr::constant(U256::from(idx)),
                output_size.clone(),
            ));
        }
        if return_data.has_symbolic_len() {
            guards.push(SymBoolExpr::cmp(
                SymBoolExprOp::Ult,
                SymExpr::constant(U256::from(idx)),
                return_data.len_expr(),
            ));
        }
        let guard = SymBoolExpr::and(guards);
        match guard.as_const() {
            Some(true) => return_data.byte(idx),
            Some(false) => self.call_output_existing_byte(dest, idx),
            None => SymExpr::ite(
                guard,
                return_data.byte(idx),
                self.call_output_existing_byte(dest, idx),
            ),
        }
    }

    pub(crate) fn call_output_existing_byte(&self, dest: &SymExpr, idx: usize) -> SymExpr {
        if let Some(dest) = dest.as_const() {
            usize::try_from(dest).map_or_else(|_| SymExpr::zero(), |dest| self.byte(dest + idx))
        } else {
            self.byte_dynamic_with_delta(dest, idx)
        }
    }

    #[cfg(test)]
    pub(crate) fn copy_memory_offset(
        &mut self,
        dest: usize,
        src: SymExpr,
        size: usize,
    ) -> Result<(), SymbolicError> {
        self.copy_memory_to_offset(SymExpr::constant(U256::from(dest)), src, size)
    }

    pub(crate) fn copy_memory_to_offset(
        &mut self,
        dest: SymExpr,
        src: SymExpr,
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
        dest: SymExpr,
        src: SymExpr,
        size: SymExpr,
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
        offset: SymExpr,
        size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes(self.read_bytes_offset(offset, size)))
    }

    pub(crate) fn return_data_symbolic_size(
        &self,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_symbolic_bytes_with_len(
            self.read_bytes_symbolic_size(offset, size.clone(), max_size),
            size,
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymCode {
    bytes: Arc<[SymExpr]>,
    jump_table: JumpTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuardedOpcode {
    End,
    Concrete(u8),
    SymbolicSize { condition: SymBoolExpr, opcode: u8 },
}

impl SymCode {
    pub(crate) fn from_symbolic_bytes(bytes: Vec<SymExpr>) -> Self {
        Self::from_shared_bytes(bytes.into())
    }

    pub(crate) fn from_shared_bytes(bytes: Arc<[SymExpr]>) -> Self {
        let analysis = bytes
            .iter()
            .map(|byte| byte.as_const().map_or(opcode::STOP, |value| value.to::<u8>()))
            .collect::<Vec<_>>();
        let analyzed = Bytecode::new_legacy(Bytes::from(analysis));
        let jump_table = analyzed.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn concrete(bytes: Vec<u8>) -> Self {
        Self::from_bytecode(&Bytecode::new_legacy(Bytes::from(bytes)))
    }

    pub(crate) fn from_bytecode(bytecode: &Bytecode) -> Self {
        let bytes = bytecode
            .original_byte_slice()
            .iter()
            .copied()
            .map(|byte| SymExpr::constant(U256::from(byte)))
            .collect::<Vec<_>>()
            .into();
        let jump_table = bytecode.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn from_memory_offset(memory: &SymMemory, offset: SymExpr, size: usize) -> Self {
        Self::from_symbolic_bytes(memory.read_bytes_offset(offset, size))
    }

    pub(crate) fn from_memory_symbolic_size(
        memory: &SymMemory,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Self {
        Self::from_symbolic_bytes(memory.read_bytes_symbolic_size(offset, size, max_size))
    }

    pub(crate) fn bytes(&self) -> &[SymExpr] {
        &self.bytes
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub(crate) const fn jump_table(&self) -> &JumpTable {
        &self.jump_table
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
                if let SymExprKind::Ite(condition, then_expr, else_expr) = byte.kind()
                    && else_expr.as_const().is_some_and(|value| value.is_zero())
                {
                    match then_expr.as_const() {
                        Some(value) if value.is_zero() => Ok(GuardedOpcode::Concrete(0)),
                        Some(value) => Ok(GuardedOpcode::SymbolicSize {
                            condition: condition.clone(),
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

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> Vec<SymExpr> {
        (0..size)
            .map(|idx| self.bytes.get(offset + idx).cloned().unwrap_or_else(SymExpr::zero))
            .collect()
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> Vec<SymExpr> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return vec![SymExpr::zero(); size];
            };
            self.read_bytes(offset, size)
        } else {
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.len()).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                self.bytes[candidate].clone(),
                result,
            );
        }
        result
    }

    pub(crate) fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        self.concrete_range(0, self.len(), reason)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SymReturnData {
    len_word: SymExpr,
    bytes: Arc<[SymExpr]>,
}

impl Default for SymReturnData {
    fn default() -> Self {
        Self { len_word: SymExpr::zero(), bytes: Vec::new().into() }
    }
}

impl SymReturnData {
    pub(crate) fn from_words(words: Vec<SymExpr>) -> Self {
        let bytes = words.into_iter().flat_map(word_bytes).collect::<Vec<_>>();
        Self::from_symbolic_bytes(bytes)
    }

    pub(crate) fn from_concrete_bytes(bytes: Vec<u8>) -> Self {
        Self::from_symbolic_bytes(
            bytes.into_iter().map(|byte| SymExpr::constant(U256::from(byte))).collect(),
        )
    }

    pub(crate) fn from_symbolic_bytes(bytes: Vec<SymExpr>) -> Self {
        let len = bytes.len();
        Self { len_word: SymExpr::constant(U256::from(len)), bytes: bytes.into() }
    }

    pub(crate) fn from_symbolic_bytes_with_len(bytes: Vec<SymExpr>, len_word: SymExpr) -> Self {
        Self { len_word, bytes: bytes.into() }
    }

    pub(crate) fn len_word(&self) -> SymExpr {
        self.len_word.clone()
    }

    pub(crate) fn len(&self) -> usize {
        self.bytes.len()
    }

    pub(crate) fn len_expr(&self) -> SymExpr {
        self.len_word.clone()
    }

    pub(crate) fn has_symbolic_len(&self) -> bool {
        self.len_word.as_const().is_none()
    }

    pub(crate) fn byte(&self, offset: usize) -> SymExpr {
        self.bytes.get(offset).cloned().unwrap_or_else(SymExpr::zero)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> Vec<SymExpr> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return vec![SymExpr::zero(); size];
            };
            (0..size).map(|idx| self.byte(offset + idx)).collect()
        } else {
            (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect()
        }
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.len()).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate - delta))),
                self.bytes[candidate].clone(),
                result,
            );
        }
        result
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymExpr, SymbolicError> {
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
