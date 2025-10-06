use revm::bytecode::{OpCode, opcode};
use std::{fmt, slice};

/// An iterator that yields opcodes and their immediate data.
///
/// If the bytecode is not well-formed, the iterator will still yield opcodes, but the immediate
/// data may be incorrect. For example, if the bytecode is `PUSH2 0x69`, the iterator will yield
/// `PUSH2, &[]`.
#[derive(Clone, Debug)]
pub struct InstIter<'a> {
    iter: slice::Iter<'a, u8>,
}

impl fmt::Display for InstIter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, op) in self.clone().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{op}")?;
        }
        Ok(())
    }
}

impl<'a> InstIter<'a> {
    /// Create a new iterator over the given bytecode slice.
    #[inline]
    pub fn new(slice: &'a [u8]) -> Self {
        Self { iter: slice.iter() }
    }

    /// Returns a new iterator that also yields the program counter alongside the opcode and
    /// immediate data.
    #[inline]
    pub fn with_pc(self) -> InstIterWithPc<'a> {
        InstIterWithPc { iter: self, pc: 0 }
    }

    /// Returns the inner iterator.
    #[inline]
    pub fn inner(&self) -> &slice::Iter<'a, u8> {
        &self.iter
    }

    /// Returns the inner iterator.
    #[inline]
    pub fn inner_mut(&mut self) -> &mut slice::Iter<'a, u8> {
        &mut self.iter
    }

    /// Returns the inner iterator.
    #[inline]
    pub fn into_inner(self) -> slice::Iter<'a, u8> {
        self.iter
    }
}

impl<'a> Iterator for InstIter<'a> {
    type Item = Inst<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|&opcode| {
            let opcode = unsafe { OpCode::new_unchecked(opcode) };
            let len = imm_len(opcode.get()) as usize;
            let (immediate, rest) = self.iter.as_slice().split_at_checked(len).unwrap_or_default();
            self.iter = rest.iter();
            Inst { opcode, immediate }
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.iter.len();
        ((len != 0) as usize, Some(len))
    }
}

impl std::iter::FusedIterator for InstIter<'_> {}

/// A bytecode iterator that yields opcodes and their immediate data, alongside the program counter.
///
/// Created by calling [`InstIter::with_pc`].
#[derive(Debug)]
pub struct InstIterWithPc<'a> {
    iter: InstIter<'a>,
    pc: usize,
}

impl<'a> Iterator for InstIterWithPc<'a> {
    type Item = (usize, Inst<'a>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next().map(|inst| {
            let pc = self.pc;
            self.pc += 1 + inst.immediate.len();
            (pc, inst)
        })
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        self.iter.size_hint()
    }
}

impl std::iter::FusedIterator for InstIterWithPc<'_> {}

/// An opcode and its immediate data. Returned by [`InstIter`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Inst<'a> {
    /// The opcode.
    pub opcode: OpCode,
    /// The immediate data, if any.
    ///
    /// If an opcode is missing immediate data, e.g. malformed or bytecode hash, this will be an
    /// empty slice.
    pub immediate: &'a [u8],
}

impl fmt::Debug for Inst<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for Inst<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.opcode)?;
        match self.immediate {
            [] => Ok(()),
            imm => write!(f, " {:#x}", alloy_primitives::hex::display(imm)),
        }
    }
}

/// Returns the length of the immediate data for the given opcode, or `0` if none.
#[inline]
const fn imm_len(op: u8) -> u8 {
    match op {
        opcode::PUSH1..=opcode::PUSH32 => op - opcode::PUSH0,
        _ => 0,
    }
}

/// Returns a string representation of the given bytecode.
pub fn format_bytecode(bytecode: &[u8]) -> String {
    let mut w = String::new();
    format_bytecode_to(bytecode, &mut w).unwrap();
    w
}

/// Formats an EVM bytecode to the given writer.
pub fn format_bytecode_to<W: fmt::Write + ?Sized>(bytecode: &[u8], w: &mut W) -> fmt::Result {
    write!(w, "{}", InstIter::new(bytecode))
}

#[cfg(test)]
mod tests {
    use super::*;
    use revm::bytecode::opcode as op;

    fn o(op: u8) -> OpCode {
        unsafe { OpCode::new_unchecked(op) }
    }

    #[test]
    fn iter_basic() {
        let bytecode = [0x01, 0x02, 0x03, 0x04, 0x05];
        let mut iter = InstIter::new(&bytecode);

        assert_eq!(iter.next(), Some(Inst { opcode: o(0x01), immediate: &[] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(0x02), immediate: &[] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(0x03), immediate: &[] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(0x04), immediate: &[] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(0x05), immediate: &[] }));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn iter_with_imm() {
        let bytecode = [op::PUSH0, op::PUSH1, 0x69, op::PUSH2, 0x01, 0x02];
        let mut iter = InstIter::new(&bytecode);

        assert_eq!(iter.next(), Some(Inst { opcode: o(op::PUSH0), immediate: &[] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(op::PUSH1), immediate: &[0x69] }));
        assert_eq!(iter.next(), Some(Inst { opcode: o(op::PUSH2), immediate: &[0x01, 0x02] }));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn iter_with_imm_too_short() {
        let bytecode = [op::PUSH2, 0x69];
        let mut iter = InstIter::new(&bytecode);

        assert_eq!(iter.next(), Some(Inst { opcode: o(op::PUSH2), immediate: &[] }));
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn display() {
        let bytecode = [op::PUSH0, op::PUSH1, 0x69, op::PUSH2, 0x01, 0x02];
        let s = format_bytecode(&bytecode);
        assert_eq!(s, "PUSH0 PUSH1 0x69 PUSH2 0x0102");
    }

    #[test]
    fn decode_push2_and_stop() {
        // 0x61 0xAA 0xBB = PUSH2 0xAABB
        // 0x00           = STOP
        let code = vec![0x61, 0xAA, 0xBB, 0x00];
        let insns = InstIter::new(&code).with_pc().collect::<Vec<_>>();

        // PUSH2 then STOP
        assert_eq!(insns.len(), 2);

        // PUSH2 at pc = 0
        let i0 = &insns[0];
        assert_eq!(i0.0, 0);
        assert_eq!(i0.1.opcode, op::PUSH2);
        assert_eq!(i0.1.immediate, &[0xAA, 0xBB]);

        // STOP at pc = 3
        let i1 = &insns[1];
        assert_eq!(i1.0, 3);
        assert_eq!(i1.1.opcode, op::STOP);
        assert!(i1.1.immediate.is_empty());
    }

    #[test]
    fn decode_arithmetic_ops() {
        // 0x01 = ADD, 0x02 = MUL, 0x03 = SUB, 0x04 = DIV
        let code = vec![0x01, 0x02, 0x03, 0x04];
        let insns = InstIter::new(&code).with_pc().collect::<Vec<_>>();

        assert_eq!(insns.len(), 4);

        let expected = [(0, op::ADD), (1, op::MUL), (2, op::SUB), (3, op::DIV)];
        for ((pc, want_op), insn) in expected.iter().zip(insns.iter()) {
            assert_eq!(insn.0, *pc);
            assert_eq!(insn.1.opcode, *want_op);
            assert!(insn.1.immediate.is_empty());
        }
    }
}
