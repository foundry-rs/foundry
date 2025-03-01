use crate::{codec::Encode, util::PartialBuffer};

use std::io::Result;

#[derive(Debug)]
pub struct LzmaEncoder {
    inner: crate::codec::Xz2Encoder,
}

impl LzmaEncoder {
    pub fn new(level: u32) -> Self {
        Self {
            inner: crate::codec::Xz2Encoder::new(crate::codec::Xz2FileFormat::Lzma, level),
        }
    }
}

impl Encode for LzmaEncoder {
    fn encode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<()> {
        self.inner.encode(input, output)
    }

    fn flush(
        &mut self,
        _output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        // Flush on LZMA 1 is not supported
        Ok(true)
    }

    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        self.inner.finish(output)
    }
}
