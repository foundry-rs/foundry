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
    symbolic_writes: Vec<SymbolicMemoryWrite>,
    size: usize,
}

#[derive(Clone, Debug)]
struct SymbolicMemoryWrite {
    offset: SymExpr,
    bytes: SymBytes,
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

    fn concrete_offset(&self) -> Option<usize> {
        self.offset.eval().and_then(|offset| usize::try_from(offset).ok())
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
        self.store_bytes(offset, value.into_bytes());
    }

    pub(crate) fn store_word_offset(&mut self, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_word(offset, value);
            }
        } else {
            self.store_symbolic_bytes(offset, value.into_bytes());
        }
    }

    pub(crate) fn store_byte(&mut self, offset: usize, value: SymExpr) {
        self.store_bytes(offset, SymBytes::exprs(vec![value.low_byte()]));
    }

    pub(crate) fn store_byte_offset(&mut self, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_byte(offset, value);
            }
        } else {
            self.store_symbolic_bytes(offset, SymBytes::exprs(vec![value.low_byte()]));
        }
    }

    pub(crate) fn store_bytes(&mut self, offset: usize, bytes: SymBytes) {
        if bytes.is_empty() {
            return;
        }
        self.size = self.size.max(Self::size_after_access(offset, bytes.len()));
        self.store_symbolic_bytes(SymExpr::constant(U256::from(offset)), bytes);
    }

    pub(crate) fn store_symbolic_bytes(&mut self, offset: SymExpr, bytes: SymBytes) {
        if bytes.is_empty() {
            return;
        }
        self.symbolic_writes.push(SymbolicMemoryWrite { offset, bytes });
    }

    pub(crate) fn store_bytes_offset(&mut self, offset: SymExpr, bytes: SymBytes) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_bytes(offset, bytes);
            }
        } else {
            self.store_symbolic_bytes(offset, bytes);
        }
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymExpr, SymbolicError> {
        Ok(self.read_bytes_offset(SymExpr::constant(U256::from(offset)), 32).word_at(0))
    }

    pub(crate) fn load_word_offset(&self, offset: SymExpr) -> Result<SymExpr, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else { return Ok(SymExpr::zero()) };
            self.load_word(offset)
        } else {
            self.load_word_dynamic(&offset)
        }
    }

    fn load_word_dynamic(&self, offset: &SymExpr) -> Result<SymExpr, SymbolicError> {
        let mut result = SymExpr::zero();
        for candidate in (0..self.size).rev() {
            result = SymExpr::ite(
                SymBoolExpr::eq(offset.clone(), SymExpr::constant(U256::from(candidate))),
                self.load_word(candidate)?,
                result,
            );
        }
        Ok(result)
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

    pub(crate) fn read_byte_exprs(&self, offset: usize, size: usize) -> Vec<SymExpr> {
        self.read_bytes(offset, size).materialize()
    }

    pub(crate) fn read_byte_exprs_offset(&self, offset: SymExpr, size: usize) -> Vec<SymExpr> {
        self.read_bytes_offset(offset, size).materialize()
    }

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> SymBytes {
        self.read_bytes_offset(SymExpr::constant(U256::from(offset)), size)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> SymBytes {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return SymBytes::concrete(vec![0; size]);
            };
            if let Some(bytes) = self.read_stored_bytes(offset, size) {
                return bytes;
            }
            SymBytes::exprs((0..size).map(|idx| self.byte(offset + idx)).collect())
        } else {
            SymBytes::exprs(
                (0..size).map(|idx| self.byte_dynamic_with_delta(&offset, idx)).collect(),
            )
        }
    }

    fn read_stored_bytes(&self, offset: usize, size: usize) -> Option<SymBytes> {
        if size == 0 {
            return Some(SymBytes::default());
        }
        let end = offset.checked_add(size)?;

        let mut unresolved = vec![(offset, end)];
        let mut pieces = Vec::new();

        for write in self.symbolic_writes.iter().rev() {
            if unresolved.is_empty() {
                break;
            }

            let write_offset = write.concrete_offset()?;
            let write_end = write_offset.checked_add(write.bytes.len())?;

            if write_end <= offset || end <= write_offset {
                continue;
            }

            let mut next_unresolved = Vec::new();
            for (start, end) in unresolved {
                let overlap_start = start.max(write_offset);
                let overlap_end = end.min(write_end);

                if overlap_start >= overlap_end {
                    next_unresolved.push((start, end));
                    continue;
                }

                if start < overlap_start {
                    next_unresolved.push((start, overlap_start));
                }

                pieces.push((
                    overlap_start - offset,
                    write
                        .bytes
                        .slice_concrete(overlap_start - write_offset, overlap_end - overlap_start),
                ));

                if overlap_end < end {
                    next_unresolved.push((overlap_end, end));
                }
            }
            unresolved = next_unresolved;
        }

        pieces.extend(
            unresolved
                .into_iter()
                .map(|(start, end)| (start - offset, SymBytes::concrete(vec![0; end - start]))),
        );
        pieces.sort_by_key(|(offset, _)| *offset);

        Some(SymBytes::concat(pieces.into_iter().map(|(_, bytes)| bytes)))
    }

    pub(crate) fn read_byte_exprs_symbolic_size(
        &self,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes_symbolic_size(offset, size, max_size).materialize()
    }

    pub(crate) fn read_bytes_symbolic_size(
        &self,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> SymBytes {
        if let Some(size) = size.eval() {
            let size = usize::try_from(size).map_or(max_size, |size| size.min(max_size));
            return SymBytes::concat([
                self.read_bytes_offset(offset, size),
                SymBytes::concrete(vec![0; max_size - size]),
            ]);
        }

        SymBytes::sized(self.read_bytes_offset(offset, max_size), size, max_size)
    }

    pub(crate) fn byte(&self, offset: usize) -> SymExpr {
        let mut result = SymExpr::zero();
        for write in &self.symbolic_writes {
            for idx in 0..write.bytes.len() {
                result = SymExpr::ite(
                    SymBoolExpr::eq(
                        SymExpr::add_const(write.offset.clone(), U256::from(idx)),
                        SymExpr::constant(U256::from(offset)),
                    ),
                    write.bytes.byte(idx),
                    result,
                );
            }
        }
        result
    }

    pub(crate) fn byte_dynamic_with_delta(&self, offset: &SymExpr, delta: usize) -> SymExpr {
        let mut result = SymExpr::constant(U256::ZERO);
        for candidate in (delta..self.size).rev() {
            let mut candidate_result = SymExpr::zero();
            for write in &self.symbolic_writes {
                for idx in 0..write.bytes.len() {
                    candidate_result = SymExpr::ite(
                        SymBoolExpr::eq(
                            SymExpr::add_const(write.offset.clone(), U256::from(idx)),
                            SymExpr::constant(U256::from(candidate)),
                        ),
                        write.bytes.byte(idx),
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
            if write.concrete_offset().is_some() {
                continue;
            }
            size = Self::max_size_word(size, write.size_after_access());
        }
        size
    }

    pub(crate) fn copy_bytes_offset(&mut self, dest: SymExpr, src: SymBytes) {
        self.store_bytes_offset(dest, src);
    }

    pub(crate) fn copy_bytes_size_offset(
        &mut self,
        dest: SymExpr,
        size: SymExpr,
        src: SymBytes,
    ) -> Result<(), SymbolicError> {
        if src.is_empty() {
            return Ok(());
        }
        if let Some(size) = size.eval() {
            let size = usize::try_from(size).map_or(src.len(), |size| size.min(src.len()));
            if size != 0 {
                self.store_bytes_offset(dest, src.slice_concrete(0, size));
            }
            return Ok(());
        }

        if let Some(dest) = dest.as_const() {
            if let Ok(dest) = usize::try_from(dest) {
                let bytes = (0..src.len())
                    .map(|idx| self.copy_size_byte_at(dest + idx, idx, &size, src.byte(idx)))
                    .collect();
                self.store_bytes(dest, SymBytes::exprs(bytes));
            }
        } else {
            let bytes = SymBytes::exprs(
                (0..src.len())
                    .map(|idx| {
                        let existing = self.byte_dynamic_with_delta(&dest, idx);
                        Self::copy_size_byte(idx, &size, src.byte(idx), existing)
                    })
                    .collect(),
            );
            self.store_symbolic_bytes(dest, bytes);
        }
        Ok(())
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
                self.copy_bytes_offset(dest, SymBytes::concrete(vec![0; size]));
                return Ok(());
            };
            self.store_bytes_offset(
                dest,
                calldata.read_bytes_offset(SymExpr::constant(U256::from(offset)), size),
            );
        } else {
            self.store_bytes_offset(dest, calldata.read_bytes_offset(offset, size));
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
        let bytes = if let Some(offset) = offset.as_const()
            && let Ok(offset) = usize::try_from(offset)
        {
            calldata.read_bytes_offset(SymExpr::constant(U256::from(offset)), max_size)
        } else {
            calldata.read_bytes_offset(offset, max_size)
        };
        self.copy_bytes_size_offset(dest, size, bytes)
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
        self.copy_bytes_size_offset(dest, size, bytes)
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
                        self.store_bytes_offset(dest, SymBytes::exprs(bytes));
                    } else {
                        self.store_bytes_offset(
                            dest,
                            return_data.read_bytes_offset(SymExpr::zero(), size),
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
                    self.store_bytes_offset(dest, SymBytes::exprs(bytes));
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
        self.copy_bytes_size_offset(dest, size, source)
    }

    pub(crate) fn return_data(
        &self,
        offset: SymExpr,
        size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_bytes(self.read_bytes_offset(offset, size)))
    }

    pub(crate) fn return_data_symbolic_size(
        &self,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_bytes_with_len(
            self.read_bytes_symbolic_size(offset, size.clone(), max_size),
            size,
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SymCode {
    bytes: SymBytes,
    jump_table: JumpTable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuardedOpcode {
    End,
    Concrete(u8),
    SymbolicSize { condition: SymBoolExpr, opcode: u8 },
}

impl SymCode {
    pub(crate) fn from_byte_exprs(bytes: Vec<SymExpr>) -> Self {
        Self::from_bytes(SymBytes::exprs(bytes))
    }

    pub(crate) fn from_bytes(bytes: SymBytes) -> Self {
        let analysis = if let Some(bytes) = bytes.as_concrete_slice() {
            bytes.to_vec()
        } else {
            (0..bytes.len())
                .map(|idx| {
                    bytes.byte(idx).as_const().map_or(opcode::STOP, |value| value.to::<u8>())
                })
                .collect::<Vec<_>>()
        };
        let analyzed = Bytecode::new_legacy(Bytes::from(analysis));
        let jump_table = analyzed.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn concrete(bytes: Vec<u8>) -> Self {
        Self::from_bytecode(&Bytecode::new_legacy(Bytes::from(bytes)))
    }

    pub(crate) fn from_bytecode(bytecode: &Bytecode) -> Self {
        let bytes = SymBytes::concrete(bytecode.original_byte_slice().to_vec());
        let jump_table = bytecode.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn from_memory_offset(memory: &SymMemory, offset: SymExpr, size: usize) -> Self {
        Self::from_bytes(memory.read_bytes_offset(offset, size))
    }

    pub(crate) fn from_memory_symbolic_size(
        memory: &SymMemory,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Self {
        Self::from_bytes(memory.read_bytes_symbolic_size(offset, size, max_size))
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
        if pc >= self.len() {
            return Ok(None);
        }
        match self.bytes.byte(pc).as_const() {
            Some(value) => Ok(Some(value.to::<u8>())),
            None => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
        }
    }

    pub(crate) fn guarded_opcode(&self, pc: usize) -> Result<GuardedOpcode, SymbolicError> {
        if pc >= self.len() {
            return Ok(GuardedOpcode::End);
        }
        let byte = self.bytes.byte(pc);
        match byte.as_const() {
            Some(value) => Ok(GuardedOpcode::Concrete(value.to::<u8>())),
            None => {
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
        if let Some(bytes) = self.bytes.as_concrete_slice() {
            let mut out = Vec::with_capacity(size);
            let end = offset.saturating_add(size).min(bytes.len());
            if offset < end {
                out.extend_from_slice(&bytes[offset..end]);
            }
            out.resize(size, 0);
            return Ok(out);
        }

        let mut out = Vec::with_capacity(size);
        for idx in 0..size {
            if offset + idx >= self.len() {
                out.push(0);
                continue;
            }
            match self.bytes.byte(offset + idx).as_const() {
                Some(value) => out.push(value.to::<u8>()),
                None => return Err(SymbolicError::Unsupported(reason)),
            }
        }
        Ok(out)
    }

    pub(crate) fn read_byte_exprs(&self, offset: usize, size: usize) -> Vec<SymExpr> {
        self.read_bytes(offset, size).materialize()
    }

    pub(crate) fn read_byte_exprs_offset(&self, offset: SymExpr, size: usize) -> Vec<SymExpr> {
        self.read_bytes_offset(offset, size).materialize()
    }

    pub(crate) fn read_bytes(&self, offset: usize, size: usize) -> SymBytes {
        self.bytes.slice_concrete(offset, size)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> SymBytes {
        self.bytes.read_offset(offset, size)
    }

    pub(crate) fn push_data_word(&self, offset: usize, len: usize) -> SymExpr {
        self.bytes.right_aligned_word(offset, len)
    }

    pub(crate) fn concrete_bytes(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        self.concrete_range(0, self.len(), reason)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SymReturnData {
    len_word: SymExpr,
    bytes: SymBytes,
}

impl Default for SymReturnData {
    fn default() -> Self {
        Self { len_word: SymExpr::zero(), bytes: SymBytes::default() }
    }
}

impl SymReturnData {
    pub(crate) fn from_words(words: Vec<SymExpr>) -> Self {
        let bytes = SymBytes::concat(words.into_iter().map(SymExpr::into_bytes));
        Self::from_bytes(bytes)
    }

    pub(crate) fn from_concrete_bytes(bytes: Vec<u8>) -> Self {
        Self::from_bytes(SymBytes::concrete(bytes))
    }

    pub(crate) fn from_byte_exprs(bytes: Vec<SymExpr>) -> Self {
        Self::from_bytes(SymBytes::exprs(bytes))
    }

    pub(crate) fn from_bytes(bytes: SymBytes) -> Self {
        let len = bytes.len();
        Self { len_word: SymExpr::constant(U256::from(len)), bytes }
    }

    pub(crate) const fn from_bytes_with_len(bytes: SymBytes, len_word: SymExpr) -> Self {
        Self { len_word, bytes }
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
        self.bytes.byte(offset)
    }

    pub(crate) fn read_bytes_offset(&self, offset: SymExpr, size: usize) -> SymBytes {
        self.bytes.read_offset(offset, size)
    }

    pub(crate) fn load_word(&self, offset: usize) -> Result<SymExpr, SymbolicError> {
        if offset.saturating_add(32) > self.len() {
            return Err(SymbolicError::Unsupported("out-of-bounds symbolic returndata word"));
        }
        Ok(self.bytes.word_at(offset))
    }

    pub(crate) fn read_concrete(&self, reason: &'static str) -> Result<Vec<u8>, SymbolicError> {
        self.bytes.concrete_bytes(reason)
    }

    pub(crate) fn to_code(&self) -> Result<SymCode, SymbolicError> {
        if self.has_symbolic_len() {
            return Err(SymbolicError::Unsupported(
                "CREATE with symbolic runtime size not modeled",
            ));
        }
        Ok(SymCode::from_bytes(self.bytes.clone()))
    }
}
