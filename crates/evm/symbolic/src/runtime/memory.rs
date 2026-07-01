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
    fn size_after_access(&self, cx: &mut SymCx) -> SymExpr {
        let len = SymExpr::constant(cx, U256::from(self.bytes.len()));
        let end = SymExpr::op(cx, SymExprOp::Add, self.offset.clone(), len);
        let round = SymExpr::constant(cx, U256::from(31));
        let rounded = SymExpr::op(cx, SymExprOp::Add, end, round);
        let mask = SymExpr::constant(cx, !U256::from(31));
        SymExpr::op(cx, SymExprOp::And, rounded, mask)
    }

    fn concrete_offset(&self) -> Option<usize> {
        self.offset.eval().and_then(|offset| usize::try_from(offset).ok())
    }

    fn concrete_byte_index(&self, offset: usize) -> Option<usize> {
        let write_offset = self.concrete_offset()?;
        let idx = offset.checked_sub(write_offset)?;
        (idx < self.bytes.len()).then_some(idx)
    }

    fn concrete_byte(&self, cx: &mut SymCx, offset: usize) -> Option<SymExpr> {
        self.concrete_byte_index(offset).map(|idx| self.bytes.byte(cx, idx))
    }
}

impl SymMemory {
    fn size_after_access(offset: usize, len: usize) -> usize {
        let Some(end) = offset.checked_add(len) else {
            return usize::MAX & !31usize;
        };
        end.checked_add(31).map(|size| size & !31usize).unwrap_or(usize::MAX & !31usize)
    }

    fn max_size_word(cx: &mut SymCx, left: SymExpr, right: SymExpr) -> SymExpr {
        if let (Some(left_value), Some(right_value)) = (left.as_const(), right.as_const()) {
            return SymExpr::constant(cx, left_value.max(right_value));
        }
        if left == right {
            left
        } else {
            let condition = SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, left.clone(), right.clone());
            SymExpr::ite(cx, condition, right, left)
        }
    }

    pub(crate) fn store_word(&mut self, cx: &mut SymCx, offset: usize, value: SymExpr) {
        let bytes = value.into_bytes(cx);
        self.store_bytes(cx, offset, bytes);
    }

    pub(crate) fn store_word_offset(&mut self, cx: &mut SymCx, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_word(cx, offset, value);
            }
        } else {
            self.store_symbolic_bytes(offset, value.into_bytes(cx));
        }
    }

    pub(crate) fn store_byte(&mut self, cx: &mut SymCx, offset: usize, value: SymExpr) {
        let byte = value.low_byte(cx);
        let bytes = SymBytes::exprs(cx, vec![byte]);
        self.store_bytes(cx, offset, bytes);
    }

    pub(crate) fn store_byte_offset(&mut self, cx: &mut SymCx, offset: SymExpr, value: SymExpr) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_byte(cx, offset, value);
            }
        } else {
            let byte = value.low_byte(cx);
            let bytes = SymBytes::exprs(cx, vec![byte]);
            self.store_symbolic_bytes(offset, bytes);
        }
    }

    pub(crate) fn store_bytes(&mut self, cx: &mut SymCx, offset: usize, bytes: SymBytes) {
        if bytes.is_empty() {
            return;
        }
        self.size = self.size.max(Self::size_after_access(offset, bytes.len()));
        let offset = SymExpr::constant(cx, U256::from(offset));
        self.store_symbolic_bytes(offset, bytes);
    }

    pub(crate) fn store_symbolic_bytes(&mut self, offset: SymExpr, bytes: SymBytes) {
        if bytes.is_empty() {
            return;
        }
        self.symbolic_writes.push(SymbolicMemoryWrite { offset, bytes });
    }

    pub(crate) fn store_bytes_offset(&mut self, cx: &mut SymCx, offset: SymExpr, bytes: SymBytes) {
        if let Some(offset) = offset.as_const() {
            if let Ok(offset) = usize::try_from(offset) {
                self.store_bytes(cx, offset, bytes);
            }
        } else {
            self.store_symbolic_bytes(offset, bytes);
        }
    }

    pub(crate) fn load_word(
        &self,
        cx: &mut SymCx,
        offset: usize,
    ) -> Result<SymExpr, SymbolicError> {
        let offset = SymExpr::constant(cx, U256::from(offset));
        Ok(self.read_bytes_offset(cx, offset, 32).word_at(cx, 0))
    }

    pub(crate) fn load_word_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else { return Ok(SymExpr::zero(cx)) };
            self.load_word(cx, offset)
        } else {
            self.load_word_dynamic(cx, &offset)
        }
    }

    fn load_word_dynamic(
        &self,
        cx: &mut SymCx,
        offset: &SymExpr,
    ) -> Result<SymExpr, SymbolicError> {
        let mut result = SymExpr::zero(cx);
        for candidate in (0..self.size).rev() {
            let candidate_expr = SymExpr::constant(cx, U256::from(candidate));
            let condition = SymBoolExpr::eq(cx, offset.clone(), candidate_expr);
            let word = self.load_word(cx, candidate)?;
            result = SymExpr::ite(cx, condition, word, result);
        }
        Ok(result)
    }

    pub(crate) fn read_concrete(
        &self,
        cx: &mut SymCx,
        offset: usize,
        size: usize,
    ) -> Result<Vec<u8>, SymbolicError> {
        let mut out = vec![0u8; size];
        for (idx, byte) in out.iter_mut().enumerate() {
            if let Some(value) = self.byte(cx, offset + idx).as_const() {
                *byte = value.to::<u8>();
            } else {
                return Err(SymbolicError::Unsupported("symbolic memory read"));
            }
        }
        Ok(out)
    }

    pub(crate) fn read_byte_exprs(
        &self,
        cx: &mut SymCx,
        offset: usize,
        size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes(cx, offset, size).materialize(cx)
    }

    pub(crate) fn read_byte_exprs_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes_offset(cx, offset, size).materialize(cx)
    }

    pub(crate) fn read_bytes(&self, cx: &mut SymCx, offset: usize, size: usize) -> SymBytes {
        let offset = SymExpr::constant(cx, U256::from(offset));
        self.read_bytes_offset(cx, offset, size)
    }

    pub(crate) fn read_bytes_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> SymBytes {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                return SymBytes::concrete(cx, vec![0; size]);
            };
            if let Some(bytes) = self.read_stored_bytes(cx, offset, size) {
                return bytes;
            }
            let bytes = (0..size).map(|idx| self.byte(cx, offset + idx)).collect();
            SymBytes::exprs(cx, bytes)
        } else {
            let bytes =
                (0..size).map(|idx| self.byte_dynamic_with_delta(cx, &offset, idx)).collect();
            SymBytes::exprs(cx, bytes)
        }
    }

    fn read_stored_bytes(&self, cx: &mut SymCx, offset: usize, size: usize) -> Option<SymBytes> {
        if size == 0 {
            return Some(SymBytes::empty(cx));
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
                    write.bytes.slice_concrete(
                        cx,
                        overlap_start - write_offset,
                        overlap_end - overlap_start,
                    ),
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
                .map(|(start, end)| (start - offset, SymBytes::concrete(cx, vec![0; end - start]))),
        );
        pieces.sort_by_key(|(offset, _)| *offset);

        Some(SymBytes::concat(cx, pieces.into_iter().map(|(_, bytes)| bytes)))
    }

    pub(crate) fn read_byte_exprs_symbolic_size(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes_symbolic_size(cx, offset, size, max_size).materialize(cx)
    }

    pub(crate) fn read_bytes_symbolic_size(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> SymBytes {
        if let Some(size) = size.eval() {
            let size = usize::try_from(size).map_or(max_size, |size| size.min(max_size));
            let bytes = self.read_bytes_offset(cx, offset, size);
            let padding = SymBytes::concrete(cx, vec![0; max_size - size]);
            return SymBytes::concat(cx, [bytes, padding]);
        }

        let bytes = self.read_bytes_offset(cx, offset, max_size);
        SymBytes::sized(cx, bytes, size, max_size)
    }

    pub(crate) fn byte(&self, cx: &mut SymCx, offset: usize) -> SymExpr {
        let mut writes = self.symbolic_writes.as_slice();
        let mut result = if let Some(base_idx) =
            writes.iter().rposition(|write| write.concrete_byte_index(offset).is_some())
        {
            let write = &writes[base_idx];
            let byte = write.concrete_byte(cx, offset).expect("concrete byte index is present");
            writes = &writes[base_idx + 1..];
            byte
        } else {
            SymExpr::zero(cx)
        };

        for write in writes {
            if let Some(byte) = write.concrete_byte(cx, offset) {
                result = byte;
                continue;
            }
            if write.concrete_offset().is_some() {
                continue;
            }
            for idx in 0..write.bytes.len() {
                let write_offset = SymExpr::add_const(cx, write.offset.clone(), U256::from(idx));
                let offset = SymExpr::constant(cx, U256::from(offset));
                let condition = SymBoolExpr::eq(cx, write_offset, offset);
                let byte = write.bytes.byte(cx, idx);
                result = SymExpr::ite(cx, condition, byte, result);
            }
        }
        result
    }

    pub(crate) fn byte_dynamic_with_delta(
        &self,
        cx: &mut SymCx,
        offset: &SymExpr,
        delta: usize,
    ) -> SymExpr {
        let mut result = SymExpr::zero(cx);
        for candidate in (delta..self.size).rev() {
            let candidate_expr = SymExpr::constant(cx, U256::from(candidate - delta));
            let condition = SymBoolExpr::eq(cx, offset.clone(), candidate_expr);
            let byte = self.byte(cx, candidate);
            result = SymExpr::ite(cx, condition, byte, result);
        }
        result
    }

    pub(crate) fn size_word(&self, cx: &mut SymCx) -> SymExpr {
        let mut size = SymExpr::constant(cx, U256::from(self.size));
        for write in &self.symbolic_writes {
            if write.concrete_offset().is_some() {
                continue;
            }
            let write_size = write.size_after_access(cx);
            size = Self::max_size_word(cx, size, write_size);
        }
        size
    }

    pub(crate) fn copy_bytes_offset(&mut self, cx: &mut SymCx, dest: SymExpr, src: SymBytes) {
        self.store_bytes_offset(cx, dest, src);
    }

    pub(crate) fn copy_bytes_size_offset(
        &mut self,
        cx: &mut SymCx,
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
                let src = src.slice_concrete(cx, 0, size);
                self.store_bytes_offset(cx, dest, src);
            }
            return Ok(());
        }

        if let Some(dest) = dest.as_const() {
            if let Ok(dest) = usize::try_from(dest) {
                let bytes = (0..src.len())
                    .map(|idx| {
                        let source = src.byte(cx, idx);
                        self.copy_size_byte_at(cx, dest + idx, idx, &size, source)
                    })
                    .collect::<Vec<_>>();
                let bytes = SymBytes::exprs(cx, bytes);
                self.store_bytes(cx, dest, bytes);
            }
        } else {
            let bytes = (0..src.len())
                .map(|idx| {
                    let existing = self.byte_dynamic_with_delta(cx, &dest, idx);
                    let source = src.byte(cx, idx);
                    Self::copy_size_byte(cx, idx, &size, source, existing)
                })
                .collect();
            let bytes = SymBytes::exprs(cx, bytes);
            self.store_symbolic_bytes(dest, bytes);
        }
        Ok(())
    }

    pub(crate) fn copy_calldata_to_offset(
        &mut self,
        cx: &mut SymCx,
        dest: SymExpr,
        offset: SymExpr,
        size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        if let Some(offset) = offset.as_const() {
            let Ok(offset) = usize::try_from(offset) else {
                let bytes = SymBytes::concrete(cx, vec![0; size]);
                self.copy_bytes_offset(cx, dest, bytes);
                return Ok(());
            };
            let offset = SymExpr::constant(cx, U256::from(offset));
            let bytes = calldata.read_bytes_offset(cx, offset, size);
            self.store_bytes_offset(cx, dest, bytes);
        } else {
            let bytes = calldata.read_bytes_offset(cx, offset, size);
            self.store_bytes_offset(cx, dest, bytes);
        }
        Ok(())
    }

    pub(crate) fn copy_calldata_symbolic_size(
        &mut self,
        cx: &mut SymCx,
        dest: SymExpr,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
        calldata: &SymCalldata,
    ) -> Result<(), SymbolicError> {
        let bytes = if let Some(offset) = offset.as_const()
            && let Ok(offset) = usize::try_from(offset)
        {
            let offset = SymExpr::constant(cx, U256::from(offset));
            calldata.read_bytes_offset(cx, offset, max_size)
        } else {
            calldata.read_bytes_offset(cx, offset, max_size)
        };
        self.copy_bytes_size_offset(cx, dest, size, bytes)
    }

    fn copy_size_byte_at(
        &self,
        cx: &mut SymCx,
        dest: usize,
        idx: usize,
        size: &SymExpr,
        source: SymExpr,
    ) -> SymExpr {
        let existing = self.byte(cx, dest);
        Self::copy_size_byte(cx, idx, size, source, existing)
    }

    fn copy_size_byte(
        cx: &mut SymCx,
        idx: usize,
        size: &SymExpr,
        source: SymExpr,
        existing: SymExpr,
    ) -> SymExpr {
        let idx = SymExpr::constant(cx, U256::from(idx));
        let condition = SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, idx, size.clone());
        SymExpr::ite(cx, condition, source, existing)
    }

    pub(crate) fn copy_return_data_to_offset(
        &mut self,
        cx: &mut SymCx,
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
        let bytes = return_data.read_bytes_offset(cx, offset, size);
        self.store_bytes_offset(cx, dest, bytes);
        Ok(())
    }

    pub(crate) fn copy_return_data_symbolic_size(
        &mut self,
        cx: &mut SymCx,
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
        let bytes = return_data.read_bytes_offset(cx, offset, max_size);
        self.copy_bytes_size_offset(cx, dest, size, bytes)
    }

    pub(crate) fn copy_call_output_offset(
        &mut self,
        cx: &mut SymCx,
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
                            .map(|idx| self.call_output_byte(cx, &dest, idx, None, return_data))
                            .collect::<Vec<_>>();
                        let bytes = SymBytes::exprs(cx, bytes);
                        self.store_bytes_offset(cx, dest, bytes);
                    } else {
                        let offset = SymExpr::zero(cx);
                        let bytes = return_data.read_bytes_offset(cx, offset, size);
                        self.store_bytes_offset(cx, dest, bytes);
                    }
                }
            }
            BoundedCopySize::Symbolic { size, max_size } => {
                let output_size = size.clone();
                let max_size = (*max_size).min(return_data.len());
                if max_size != 0 {
                    let bytes = (0..max_size)
                        .map(|idx| {
                            self.call_output_byte(cx, &dest, idx, Some(&output_size), return_data)
                        })
                        .collect::<Vec<_>>();
                    let bytes = SymBytes::exprs(cx, bytes);
                    self.store_bytes_offset(cx, dest, bytes);
                }
            }
        }
        Ok(())
    }

    pub(crate) fn call_output_byte(
        &self,
        cx: &mut SymCx,
        dest: &SymExpr,
        idx: usize,
        output_size: Option<&SymExpr>,
        return_data: &SymReturnData,
    ) -> SymExpr {
        let guard = match (output_size, return_data.has_symbolic_len()) {
            (Some(output_size), true) => {
                let idx_expr = SymExpr::constant(cx, U256::from(idx));
                let output_guard =
                    SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, idx_expr.clone(), output_size.clone());
                let len_guard =
                    SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, idx_expr, return_data.len_expr());
                SymBoolExpr::and_pair(cx, output_guard, len_guard)
            }
            (Some(output_size), false) => {
                let idx_expr = SymExpr::constant(cx, U256::from(idx));
                SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, idx_expr, output_size.clone())
            }
            (None, true) => {
                let idx_expr = SymExpr::constant(cx, U256::from(idx));
                SymBoolExpr::cmp(cx, SymBoolExprOp::Ult, idx_expr, return_data.len_expr())
            }
            (None, false) => SymBoolExpr::constant(cx, true),
        };
        match guard.as_const() {
            Some(true) => return_data.byte(cx, idx),
            Some(false) => self.call_output_existing_byte(cx, dest, idx),
            None => {
                let byte = return_data.byte(cx, idx);
                let existing = self.call_output_existing_byte(cx, dest, idx);
                SymExpr::ite(cx, guard, byte, existing)
            }
        }
    }

    pub(crate) fn call_output_existing_byte(
        &self,
        cx: &mut SymCx,
        dest: &SymExpr,
        idx: usize,
    ) -> SymExpr {
        if let Some(dest) = dest.as_const() {
            match usize::try_from(dest) {
                Ok(dest) => self.byte(cx, dest + idx),
                Err(_) => SymExpr::zero(cx),
            }
        } else {
            self.byte_dynamic_with_delta(cx, dest, idx)
        }
    }

    pub(crate) fn copy_memory_to_offset(
        &mut self,
        cx: &mut SymCx,
        dest: SymExpr,
        src: SymExpr,
        size: usize,
    ) -> Result<(), SymbolicError> {
        if size == 0 {
            return Ok(());
        }
        let bytes = self.read_bytes_offset(cx, src, size);
        self.store_bytes_offset(cx, dest, bytes);
        Ok(())
    }

    pub(crate) fn copy_memory_symbolic_size(
        &mut self,
        cx: &mut SymCx,
        dest: SymExpr,
        src: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<(), SymbolicError> {
        if max_size == 0 {
            return Ok(());
        }
        let source = self.read_bytes_offset(cx, src, max_size);
        self.copy_bytes_size_offset(cx, dest, size, source)
    }

    pub(crate) fn return_data(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        let bytes = self.read_bytes_offset(cx, offset, size);
        Ok(SymReturnData::from_bytes(cx, bytes))
    }

    pub(crate) fn return_data_symbolic_size(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Result<SymReturnData, SymbolicError> {
        Ok(SymReturnData::from_bytes_with_len(
            self.read_bytes_symbolic_size(cx, offset, size.clone(), max_size),
            size,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
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
    pub(crate) fn empty(cx: &mut SymCx) -> Self {
        Self { bytes: SymBytes::empty(cx), jump_table: JumpTable::default() }
    }

    pub(crate) fn from_byte_exprs(cx: &mut SymCx, bytes: Vec<SymExpr>) -> Self {
        let bytes = SymBytes::exprs(cx, bytes);
        Self::from_bytes(cx, bytes)
    }

    pub(crate) fn from_bytes(cx: &mut SymCx, bytes: SymBytes) -> Self {
        let analysis = if let Some(bytes) = bytes.as_concrete_slice() {
            bytes.to_vec()
        } else {
            (0..bytes.len())
                .map(|idx| {
                    bytes.byte(cx, idx).as_const().map_or(opcode::STOP, |value| value.to::<u8>())
                })
                .collect::<Vec<_>>()
        };
        let analyzed = Bytecode::new_legacy(Bytes::from(analysis));
        let jump_table = analyzed.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn concrete(cx: &mut SymCx, bytes: Vec<u8>) -> Self {
        Self::from_bytecode(cx, &Bytecode::new_legacy(Bytes::from(bytes)))
    }

    pub(crate) fn from_bytecode(cx: &mut SymCx, bytecode: &Bytecode) -> Self {
        let bytes = SymBytes::concrete(cx, bytecode.original_byte_slice().to_vec());
        let jump_table = bytecode.legacy_jump_table().cloned().unwrap_or_default();
        Self { bytes, jump_table }
    }

    pub(crate) fn from_memory_offset(
        cx: &mut SymCx,
        memory: &SymMemory,
        offset: SymExpr,
        size: usize,
    ) -> Self {
        let bytes = memory.read_bytes_offset(cx, offset, size);
        Self::from_bytes(cx, bytes)
    }

    pub(crate) fn from_memory_symbolic_size(
        cx: &mut SymCx,
        memory: &SymMemory,
        offset: SymExpr,
        size: SymExpr,
        max_size: usize,
    ) -> Self {
        let bytes = memory.read_bytes_symbolic_size(cx, offset, size, max_size);
        Self::from_bytes(cx, bytes)
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

    pub(crate) fn opcode(&self, cx: &mut SymCx, pc: usize) -> Result<Option<u8>, SymbolicError> {
        if pc >= self.len() {
            return Ok(None);
        }
        match self.bytes.byte(cx, pc).as_const() {
            Some(value) => Ok(Some(value.to::<u8>())),
            None => Err(SymbolicError::Unsupported("symbolic bytecode opcode")),
        }
    }

    pub(crate) fn guarded_opcode(
        &self,
        cx: &mut SymCx,
        pc: usize,
    ) -> Result<GuardedOpcode, SymbolicError> {
        if pc >= self.len() {
            return Ok(GuardedOpcode::End);
        }
        let byte = self.bytes.byte(cx, pc);
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
        cx: &mut SymCx,
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
            match self.bytes.byte(cx, offset + idx).as_const() {
                Some(value) => out.push(value.to::<u8>()),
                None => return Err(SymbolicError::Unsupported(reason)),
            }
        }
        Ok(out)
    }

    pub(crate) fn read_byte_exprs(
        &self,
        cx: &mut SymCx,
        offset: usize,
        size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes(cx, offset, size).materialize(cx)
    }

    pub(crate) fn read_byte_exprs_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> Vec<SymExpr> {
        self.read_bytes_offset(cx, offset, size).materialize(cx)
    }

    pub(crate) fn read_bytes(&self, cx: &mut SymCx, offset: usize, size: usize) -> SymBytes {
        self.bytes.slice_concrete(cx, offset, size)
    }

    pub(crate) fn read_bytes_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> SymBytes {
        self.bytes.read_offset(cx, offset, size)
    }

    pub(crate) fn push_data_word(&self, cx: &mut SymCx, offset: usize, len: usize) -> SymExpr {
        self.bytes.right_aligned_word(cx, offset, len)
    }

    pub(crate) fn concrete_bytes(
        &self,
        cx: &mut SymCx,
        reason: &'static str,
    ) -> Result<Vec<u8>, SymbolicError> {
        self.concrete_range(cx, 0, self.len(), reason)
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SymReturnData {
    len_word: SymExpr,
    bytes: SymBytes,
}

impl SymReturnData {
    pub(crate) fn empty(cx: &mut SymCx) -> Self {
        Self { len_word: SymExpr::zero(cx), bytes: SymBytes::empty(cx) }
    }

    pub(crate) fn from_words(cx: &mut SymCx, words: Vec<SymExpr>) -> Self {
        let bytes = words.into_iter().map(|word| word.into_bytes(cx)).collect::<Vec<_>>();
        let bytes = SymBytes::concat(cx, bytes);
        Self::from_bytes(cx, bytes)
    }

    pub(crate) fn from_concrete_bytes(cx: &mut SymCx, bytes: Vec<u8>) -> Self {
        let bytes = SymBytes::concrete(cx, bytes);
        Self::from_bytes(cx, bytes)
    }

    pub(crate) fn from_byte_exprs(cx: &mut SymCx, bytes: Vec<SymExpr>) -> Self {
        let bytes = SymBytes::exprs(cx, bytes);
        Self::from_bytes(cx, bytes)
    }

    pub(crate) fn from_bytes(cx: &mut SymCx, bytes: SymBytes) -> Self {
        let len = bytes.len();
        Self { len_word: SymExpr::constant(cx, U256::from(len)), bytes }
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

    pub(crate) fn byte(&self, cx: &mut SymCx, offset: usize) -> SymExpr {
        self.bytes.byte(cx, offset)
    }

    pub(crate) fn read_bytes_offset(
        &self,
        cx: &mut SymCx,
        offset: SymExpr,
        size: usize,
    ) -> SymBytes {
        self.bytes.read_offset(cx, offset, size)
    }

    pub(crate) fn load_word(
        &self,
        cx: &mut SymCx,
        offset: usize,
    ) -> Result<SymExpr, SymbolicError> {
        if offset.saturating_add(32) > self.len() {
            return Err(SymbolicError::Unsupported("out-of-bounds symbolic returndata word"));
        }
        Ok(self.bytes.word_at(cx, offset))
    }

    pub(crate) fn read_concrete(
        &self,
        cx: &mut SymCx,
        reason: &'static str,
    ) -> Result<Vec<u8>, SymbolicError> {
        self.bytes.concrete_bytes(cx, reason)
    }

    pub(crate) fn to_code(&self, cx: &mut SymCx) -> Result<SymCode, SymbolicError> {
        if self.has_symbolic_len() {
            return Err(SymbolicError::Unsupported(
                "CREATE with symbolic runtime size not modeled",
            ));
        }
        Ok(SymCode::from_bytes(cx, self.bytes.clone()))
    }
}
