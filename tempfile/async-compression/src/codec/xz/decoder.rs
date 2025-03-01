use crate::{codec::Decode, util::PartialBuffer};

use std::io::{Error, ErrorKind, Result};

#[derive(Debug)]
pub struct XzDecoder {
    inner: crate::codec::Xz2Decoder,
    skip_padding: Option<u8>,
}

impl XzDecoder {
    pub fn new() -> Self {
        Self {
            inner: crate::codec::Xz2Decoder::new(u64::MAX),
            skip_padding: None,
        }
    }

    pub fn with_memlimit(memlimit: u64) -> Self {
        Self {
            inner: crate::codec::Xz2Decoder::new(memlimit),
            skip_padding: None,
        }
    }
}

impl Decode for XzDecoder {
    fn reinit(&mut self) -> Result<()> {
        self.skip_padding = Some(4);
        self.inner.reinit()
    }

    fn decode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        if let Some(ref mut count) = self.skip_padding {
            while input.unwritten().first() == Some(&0) {
                input.advance(1);
                *count -= 1;
                if *count == 0 {
                    *count = 4;
                }
            }
            if input.unwritten().is_empty() {
                return Ok(true);
            }
            // If this is non-padding then it cannot start with null bytes, so it must be invalid
            // padding
            if *count != 4 {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "stream padding was not a multiple of 4 bytes",
                ));
            }
            self.skip_padding = None;
        }
        self.inner.decode(input, output)
    }

    fn flush(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        if self.skip_padding.is_some() {
            return Ok(true);
        }
        self.inner.flush(output)
    }

    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool> {
        if self.skip_padding.is_some() {
            return Ok(true);
        }
        self.inner.finish(output)
    }
}
