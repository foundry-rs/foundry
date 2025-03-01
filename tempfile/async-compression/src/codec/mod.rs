use crate::util::PartialBuffer;
use std::io::Result;

#[cfg(feature = "brotli")]
mod brotli;
#[cfg(feature = "bzip2")]
mod bzip2;
#[cfg(feature = "deflate")]
mod deflate;
#[cfg(feature = "deflate64")]
mod deflate64;
#[cfg(feature = "flate2")]
mod flate;
#[cfg(feature = "gzip")]
mod gzip;
#[cfg(feature = "lzma")]
mod lzma;
#[cfg(feature = "xz")]
mod xz;
#[cfg(feature = "xz2")]
mod xz2;
#[cfg(feature = "zlib")]
mod zlib;
#[cfg(feature = "zstd")]
mod zstd;

#[cfg(feature = "brotli")]
pub(crate) use self::brotli::{BrotliDecoder, BrotliEncoder};
#[cfg(feature = "bzip2")]
pub(crate) use self::bzip2::{BzDecoder, BzEncoder};
#[cfg(feature = "deflate")]
pub(crate) use self::deflate::{DeflateDecoder, DeflateEncoder};
#[cfg(feature = "deflate64")]
pub(crate) use self::deflate64::Deflate64Decoder;
#[cfg(feature = "flate2")]
pub(crate) use self::flate::{FlateDecoder, FlateEncoder};
#[cfg(feature = "gzip")]
pub(crate) use self::gzip::{GzipDecoder, GzipEncoder};
#[cfg(feature = "lzma")]
pub(crate) use self::lzma::{LzmaDecoder, LzmaEncoder};
#[cfg(feature = "xz")]
pub(crate) use self::xz::{XzDecoder, XzEncoder};
#[cfg(feature = "xz2")]
pub(crate) use self::xz2::{Xz2Decoder, Xz2Encoder, Xz2FileFormat};
#[cfg(feature = "zlib")]
pub(crate) use self::zlib::{ZlibDecoder, ZlibEncoder};
#[cfg(feature = "zstd")]
pub(crate) use self::zstd::{ZstdDecoder, ZstdEncoder};

pub trait Encode {
    fn encode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<()>;

    /// Returns whether the internal buffers are flushed
    fn flush(&mut self, output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>)
        -> Result<bool>;

    /// Returns whether the internal buffers are flushed and the end of the stream is written
    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool>;
}

pub trait Decode {
    /// Reinitializes this decoder ready to decode a new member/frame of data.
    fn reinit(&mut self) -> Result<()>;

    /// Returns whether the end of the stream has been read
    fn decode(
        &mut self,
        input: &mut PartialBuffer<impl AsRef<[u8]>>,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool>;

    /// Returns whether the internal buffers are flushed
    fn flush(&mut self, output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>)
        -> Result<bool>;

    /// Returns whether the internal buffers are flushed
    fn finish(
        &mut self,
        output: &mut PartialBuffer<impl AsRef<[u8]> + AsMut<[u8]>>,
    ) -> Result<bool>;
}
