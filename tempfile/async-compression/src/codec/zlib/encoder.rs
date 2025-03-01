use crate::{codec::Encode, util::PartialBuffer};
use std::io::Result;

use flate2::Compression;

#[derive(Debug)]
pub struct ZlibEncoder {
    inner: crate::codec::FlateEncoder,
}

impl ZlibEncoder {
    pub(crate) fn new(level: Compression) -> Self {
        Self {
            inner: crate::codec::FlateEncoder::new(level, true),
        }
    }

    pub(crate) fn get_ref(&self) -> &crate::codec::FlateEncoder {
        &self.inner
    }
}

impl Encode for ZlibEncoder {
    fn encode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<()> {
        self.inner.encode(input, output)
    }

    fn flush(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        self.inner.flush(output)
    }

    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        self.inner.finish(output)
    }
}
