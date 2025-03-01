use std::{fmt, io};

use xz2::stream::{Action, Check, LzmaOptions, Status, Stream};

use crate::{
    codec::{Encode, Xz2FileFormat},
    util::PartialBuffer,
};

pub struct Xz2Encoder {
    stream: Stream,
}

impl fmt::Debug for Xz2Encoder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Xz2Encoder").finish_non_exhaustive()
    }
}

impl Xz2Encoder {
    pub fn new(format: Xz2FileFormat, level: u32) -> Self {
        let stream = match format {
            Xz2FileFormat::Xz => Stream::new_easy_encoder(level, Check::Crc64).unwrap(),
            Xz2FileFormat::Lzma => {
                Stream::new_lzma_encoder(&LzmaOptions::new_preset(level).unwrap()).unwrap()
            }
        };

        Self { stream }
    }
}

impl Encode for Xz2Encoder {
    fn encode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> io::Result<()> {
        let previous_in = self.stream.total_in() as usize;
        let previous_out = self.stream.total_out() as usize;

        let status = self
            .stream
            .process(input.unwritten(), output.unwritten_mut(), Action::Run)?;

        input.advance(self.stream.total_in() as usize - previous_in);
        output.advance(self.stream.total_out() as usize - previous_out);

        match status {
            Status::Ok | Status::StreamEnd => Ok(()),
            Status::GetCheck => Err(io::Error::new(
                io::ErrorKind::Other,
                "Unexpected lzma integrity check",
            )),
            Status::MemNeeded => Err(io::Error::new(io::ErrorKind::Other, "out of memory")),
        }
    }

    fn flush(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> io::Result<bool> {
        let previous_out = self.stream.total_out() as usize;

        let status = self
            .stream
            .process(&[], output.unwritten_mut(), Action::SyncFlush)?;

        output.advance(self.stream.total_out() as usize - previous_out);

        match status {
            Status::Ok => Ok(false),
            Status::StreamEnd => Ok(true),
            Status::GetCheck => Err(io::Error::new(
                io::ErrorKind::Other,
                "Unexpected lzma integrity check",
            )),
            Status::MemNeeded => Err(io::Error::new(io::ErrorKind::Other, "out of memory")),
        }
    }

    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> io::Result<bool> {
        let previous_out = self.stream.total_out() as usize;

        let status = self
            .stream
            .process(&[], output.unwritten_mut(), Action::Finish)?;

        output.advance(self.stream.total_out() as usize - previous_out);

        match status {
            Status::Ok => Ok(false),
            Status::StreamEnd => Ok(true),
            Status::GetCheck => Err(io::Error::new(
                io::ErrorKind::Other,
                "Unexpected lzma integrity check",
            )),
            Status::MemNeeded => Err(io::Error::new(io::ErrorKind::Other, "out of memory")),
        }
    }
}
