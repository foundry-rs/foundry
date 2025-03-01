macro_rules! algos {
    (@algo $algo:ident [$algo_s:expr] $decoder:ident $encoder:ident <$inner:ident>
        { @enc $($encoder_methods:tt)* }
        { @dec $($decoder_methods:tt)* }
    ) => {
        #[cfg(feature = $algo_s)]
        decoder! {
            #[doc = concat!("A ", $algo_s, " decoder, or decompressor")]
            #[cfg(feature = $algo_s)]
            $decoder<$inner>

            { $($decoder_methods)* }
        }

        #[cfg(feature = $algo_s)]
        encoder! {
            #[doc = concat!("A ", $algo_s, " encoder, or compressor.")]
            #[cfg(feature = $algo_s)]
            $encoder<$inner> {
                pub fn new(inner: $inner) -> Self {
                    Self::with_quality(inner, crate::Level::Default)
                }
            }

            { $($encoder_methods)* }
        }
    };

    (@algo $algo:ident [$algo_s:expr] $decoder:ident $encoder:ident <$inner:ident>
        { @dec $($decoder_methods:tt)* }
    ) => {
        #[cfg(feature = $algo_s)]
        decoder! {
            #[doc = concat!("A ", $algo_s, " decoder, or decompressor")]
            #[cfg(feature = $algo_s)]
            $decoder<$inner>

            { $($decoder_methods)* }
        }
    };

    ($($mod:ident)::+ <$inner:ident>) => {
        algos!(@algo brotli ["brotli"] BrotliDecoder BrotliEncoder <$inner>
        { @enc
            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                let params = brotli::enc::backward_references::BrotliEncoderParams::default();

                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::BrotliEncoder::new(level.into_brotli(params)),
                    ),
                }
            }

            /// Creates a new encoder, using the specified compression level and parameters, which
            /// will read uncompressed data from the given stream and emit a compressed stream.
            pub fn with_quality_and_params(
                inner: $inner,
                level: crate::Level,
                params: crate::brotli::EncoderParams,
            ) -> Self {
                let params = level.into_brotli(params.as_brotli());

                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::BrotliEncoder::new(params),
                    ),
                }
            }
        }
        { @dec }
        );

        algos!(@algo bzip2 ["bzip2"] BzDecoder BzEncoder <$inner>
        { @enc

            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::BzEncoder::new(level.into_bzip2(), 0),
                    ),
                }
            }
        }
        { @dec }
        );

        algos!(@algo deflate ["deflate"] DeflateDecoder DeflateEncoder <$inner>
        { @enc
            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::DeflateEncoder::new(level.into_flate2()),
                    ),
                }
            }
        }
        { @dec }
        );

        algos!(@algo deflate ["deflate64"] Deflate64Decoder Deflate64Encoder <$inner>
        { @dec }
        );

        algos!(@algo gzip ["gzip"] GzipDecoder GzipEncoder <$inner>
        { @enc

            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::GzipEncoder::new(level.into_flate2()),
                    ),
                }
            }
        }
        { @dec }
        );

        algos!(@algo zlib ["zlib"] ZlibDecoder ZlibEncoder <$inner>
        { @enc
            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::ZlibEncoder::new(level.into_flate2()),
                    ),
                }
            }

            /// Returns the total number of input bytes which have been processed by this compression object.
            pub fn total_in(&self) -> u64 {
                self.inner.get_encoder_ref().get_ref().get_ref().total_in()
            }

            /// Returns the total number of output bytes which have been produced by this compression object.
            pub fn total_out(&self) -> u64 {
                self.inner.get_encoder_ref().get_ref().get_ref().total_out()
            }
        }
        { @dec }
        );

        algos!(@algo zstd ["zstd"] ZstdDecoder ZstdEncoder <$inner>
        { @enc

            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::ZstdEncoder::new(level.into_zstd()),
                    ),
                }
            }

            /// Creates a new encoder, using the specified compression level and parameters, which
            /// will read uncompressed data from the given stream and emit a compressed stream.
            ///
            /// # Panics
            ///
            /// Panics if this function is called with a [`CParameter::nb_workers()`] parameter and
            /// the `zstdmt` crate feature is _not_ enabled.
            ///
            /// [`CParameter::nb_workers()`]: crate::zstd::CParameter
            //
            // TODO: remove panic note on next breaking release, along with `CParameter::nb_workers`
            // change
            pub fn with_quality_and_params(inner: $inner, level: crate::Level, params: &[crate::zstd::CParameter]) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::ZstdEncoder::new_with_params(level.into_zstd(), params),
                    ),
                }
            }

            /// Creates a new encoder, using the specified compression level and pre-trained
            /// dictionary, which will read uncompressed data from the given stream and emit a
            /// compressed stream.
            ///
            /// Dictionaries provide better compression ratios for small files, but are required to
            /// be present during decompression.
            ///
            /// # Errors
            ///
            /// Returns error when `dictionary` is not valid.
            pub fn with_dict(inner: $inner, level: crate::Level, dictionary: &[u8]) -> ::std::io::Result<Self> {
                Ok(Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::ZstdEncoder::new_with_dict(level.into_zstd(), dictionary)?,
                    ),
                })
            }
        }
        { @dec
            /// Creates a new decoder, using the specified parameters, which will read compressed
            /// data from the given stream and emit a decompressed stream.
            pub fn with_params(inner: $inner, params: &[crate::zstd::DParameter]) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Decoder::new(
                        inner,
                        crate::codec::ZstdDecoder::new_with_params(params),
                    ),
                }
            }

            /// Creates a new decoder, using the specified compression level and pre-trained
            /// dictionary, which will read compressed data from the given stream and emit an
            /// uncompressed stream.
            ///
            /// Dictionaries provide better compression ratios for small files, but are required to
            /// be present during decompression. The dictionary used must be the same as the one
            /// used for compression.
            ///
            /// # Errors
            ///
            /// Returns error when `dictionary` is not valid.
            pub fn with_dict(inner: $inner, dictionary: &[u8]) -> ::std::io::Result<Self> {
                Ok(Self {
                    inner: crate::$($mod::)+generic::Decoder::new(
                        inner,
                        crate::codec::ZstdDecoder::new_with_dict(dictionary)?,
                    ),
                })
            }
        }
        );

        algos!(@algo xz ["xz"] XzDecoder XzEncoder <$inner>
        { @enc

            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::XzEncoder::new(level.into_xz2()),
                    ),
                }
            }
        }
        { @dec
            /// Creates a new decoder with the specified limit of memory.
            ///
            /// # Errors
            ///
            /// An IO error may be returned during decoding if the specified limit is too small.
            pub fn with_mem_limit(read: $inner, memlimit: u64) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Decoder::new(
                        read,
                        crate::codec::XzDecoder::with_memlimit(memlimit),
                    ),
                }
            }
        }
        );

        algos!(@algo lzma ["lzma"] LzmaDecoder LzmaEncoder <$inner>
        { @enc

            pub fn with_quality(inner: $inner, level: crate::Level) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Encoder::new(
                        inner,
                        crate::codec::LzmaEncoder::new(level.into_xz2()),
                    ),
                }
            }
        }
        { @dec
            /// Creates a new decoder with the specified limit of memory.
            ///
            /// # Errors
            ///
            /// An IO error may be returned during decoding if the specified limit is too small.
            pub fn with_mem_limit(read: $inner, memlimit: u64) -> Self {
                Self {
                    inner: crate::$($mod::)+generic::Decoder::new(
                        read,
                        crate::codec::LzmaDecoder::with_memlimit(memlimit),
                    ),
                }
            }

        }
        );
    }
}
