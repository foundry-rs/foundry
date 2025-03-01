macro_rules! io_algo {
    ($impl:ident, $algo:ident($encoder:ident, $decoder:ident)) => {
        pub mod $impl {
            pub mod read {
                pub use crate::utils::impls::$impl::read::{poll_read, to_vec};
            }

            pub mod bufread {
                pub use crate::utils::impls::$impl::bufread::{from, AsyncBufRead};
                pub use async_compression::$impl::bufread::{
                    $decoder as Decoder, $encoder as Encoder,
                };

                use crate::utils::{pin_mut, Level};

                pub fn compress(input: impl AsyncBufRead) -> Vec<u8> {
                    pin_mut!(input);
                    super::read::to_vec(Encoder::with_quality(input, Level::Fastest))
                }

                pub fn decompress(input: impl AsyncBufRead) -> Vec<u8> {
                    pin_mut!(input);
                    super::read::to_vec(Decoder::new(input))
                }
            }

            pub mod write {
                pub use crate::utils::impls::$impl::write::to_vec;
                pub use async_compression::$impl::write::{
                    $decoder as Decoder, $encoder as Encoder,
                };

                use crate::utils::Level;

                pub fn compress(input: &[Vec<u8>], limit: usize) -> Vec<u8> {
                    to_vec(
                        input,
                        |input| Box::pin(Encoder::with_quality(input, Level::Fastest)),
                        limit,
                    )
                }

                pub fn decompress(input: &[Vec<u8>], limit: usize) -> Vec<u8> {
                    to_vec(input, |input| Box::pin(Decoder::new(input)), limit)
                }
            }
        }
    };
}

macro_rules! algos {
    ($(pub mod $name:ident($feat:literal, $encoder:ident, $decoder:ident) { pub mod sync { $($tt:tt)* } })*) => {
        $(
            #[cfg(feature = $feat)]
            pub mod $name {
                pub mod sync { $($tt)* }

                #[cfg(feature = "futures-io")]
                io_algo!(futures, $name($encoder, $decoder));

                #[cfg(feature = "tokio")]
                io_algo!(tokio, $name($encoder, $decoder));
            }
        )*
    }
}

algos! {
    pub mod brotli("brotli", BrotliEncoder, BrotliDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use brotli::{enc::backward_references::BrotliEncoderParams, CompressorReader};
                let params = BrotliEncoderParams { quality: 1, ..Default::default() };
                to_vec(CompressorReader::with_params(bytes, 0, &params))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use brotli::Decompressor;
                to_vec(Decompressor::new(bytes, 0))
            }
        }
    }

    pub mod bzip2("bzip2", BzEncoder, BzDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use bzip2::{bufread::BzEncoder, Compression};
                to_vec(BzEncoder::new(bytes, Compression::fast()))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use bzip2::bufread::BzDecoder;
                to_vec(BzDecoder::new(bytes))
            }
        }
    }

    pub mod deflate("deflate", DeflateEncoder, DeflateDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use flate2::{bufread::DeflateEncoder, Compression};
                to_vec(DeflateEncoder::new(bytes, Compression::fast()))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use flate2::bufread::DeflateDecoder;
                to_vec(DeflateDecoder::new(bytes))
            }
        }
    }

    pub mod zlib("zlib", ZlibEncoder, ZlibDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use flate2::{bufread::ZlibEncoder, Compression};
                to_vec(ZlibEncoder::new(bytes, Compression::fast()))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use flate2::bufread::ZlibDecoder;
                to_vec(ZlibDecoder::new(bytes))
            }
        }
    }

    pub mod gzip("gzip", GzipEncoder, GzipDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use flate2::{bufread::GzEncoder, Compression};
                to_vec(GzEncoder::new(bytes, Compression::fast()))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use flate2::bufread::GzDecoder;
                to_vec(GzDecoder::new(bytes))
            }
        }
    }

    pub mod zstd("zstd", ZstdEncoder, ZstdDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use libzstd::stream::read::Encoder;
                use libzstd::DEFAULT_COMPRESSION_LEVEL;
                to_vec(Encoder::new(bytes, DEFAULT_COMPRESSION_LEVEL).unwrap())
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use libzstd::stream::read::Decoder;
                to_vec(Decoder::new(bytes).unwrap())
            }
        }
    }

    pub mod xz("xz", XzEncoder, XzDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use xz2::bufread::XzEncoder;

                to_vec(XzEncoder::new(bytes, 0))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use xz2::bufread::XzDecoder;

                to_vec(XzDecoder::new(bytes))
            }
        }
    }

    pub mod lzma("lzma", LzmaEncoder, LzmaDecoder) {
        pub mod sync {
            pub use crate::utils::impls::sync::to_vec;

            pub fn compress(bytes: &[u8]) -> Vec<u8> {
                use xz2::bufread::XzEncoder;
                use xz2::stream::{LzmaOptions, Stream};

                to_vec(XzEncoder::new_stream(
                    bytes,
                    Stream::new_lzma_encoder(&LzmaOptions::new_preset(0).unwrap()).unwrap(),
                ))
            }

            pub fn decompress(bytes: &[u8]) -> Vec<u8> {
                use xz2::bufread::XzDecoder;
                use xz2::stream::Stream;

                to_vec(XzDecoder::new_stream(
                    bytes,
                    Stream::new_lzma_decoder(u64::MAX).unwrap(),
                ))
            }
        }
    }
}
