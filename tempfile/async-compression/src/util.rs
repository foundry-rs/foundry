pub fn _assert_send<T: Send>() {}
pub fn _assert_sync<T: Sync>() {}

#[derive(Debug, Default)]
pub struct PartialBuffer<B: AsRef<[u8]>> {
    buffer: B,
    index: usize,
}

impl<B: AsRef<[u8]>> PartialBuffer<B> {
    pub(crate) fn new(buffer: B) -> Self {
        Self { buffer, index: 0 }
    }

    pub(crate) fn written(&self) -> &[u8] {
        &self.buffer.as_ref()[..self.index]
    }

    pub(crate) fn unwritten(&self) -> &[u8] {
        &self.buffer.as_ref()[self.index..]
    }

    pub(crate) fn advance(&mut self, amount: usize) {
        self.index += amount;
    }

    pub(crate) fn get_mut(&mut self) -> &mut B {
        &mut self.buffer
    }

    pub(crate) fn into_inner(self) -> B {
        self.buffer
    }
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> PartialBuffer<B> {
    pub(crate) fn unwritten_mut(&mut self) -> &mut [u8] {
        &mut self.buffer.as_mut()[self.index..]
    }

    pub(crate) fn copy_unwritten_from<C: AsRef<[u8]>>(&mut self, other: &mut PartialBuffer<C>) {
        let len = std::cmp::min(self.unwritten().len(), other.unwritten().len());

        self.unwritten_mut()[..len].copy_from_slice(&other.unwritten()[..len]);

        self.advance(len);
        other.advance(len);
    }
}

impl<B: AsRef<[u8]> + Default> PartialBuffer<B> {
    pub(crate) fn take(&mut self) -> Self {
        std::mem::replace(self, Self::new(B::default()))
    }
}

impl<B: AsRef<[u8]> + AsMut<[u8]>> From<B> for PartialBuffer<B> {
    fn from(buffer: B) -> Self {
        Self::new(buffer)
    }
}
