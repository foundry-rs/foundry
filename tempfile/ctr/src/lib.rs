//! Generic implementations of [CTR mode][1] for block ciphers.
//!
//! <img src="https://raw.githubusercontent.com/RustCrypto/media/26acc39f/img/block-modes/ctr_enc.svg" width="49%" />
//! <img src="https://raw.githubusercontent.com/RustCrypto/media/26acc39f/img/block-modes/ctr_dec.svg" width="49%"/>
//!
//! Mode functionality is accessed using traits from re-exported [`cipher`] crate.
//!
//! # ⚠️ Security Warning: Hazmat!
//!
//! This crate does not ensure ciphertexts are authentic! Thus ciphertext integrity
//! is not verified, which can lead to serious vulnerabilities!
//!
//! # Example
//! ```
//! use aes::cipher::{KeyIvInit, StreamCipher, StreamCipherSeek};
//! use hex_literal::hex;
//!
//! type Aes128Ctr64LE = ctr::Ctr64LE<aes::Aes128>;
//!
//! let key = [0x42; 16];
//! let iv = [0x24; 16];
//! let plaintext = *b"hello world! this is my plaintext.";
//! let ciphertext = hex!(
//!     "3357121ebb5a29468bd861467596ce3da59bdee42dcc0614dea955368d8a5dc0cad4"
//! );
//!
//! // encrypt in-place
//! let mut buf = plaintext.to_vec();
//! let mut cipher = Aes128Ctr64LE::new(&key.into(), &iv.into());
//! cipher.apply_keystream(&mut buf);
//! assert_eq!(buf[..], ciphertext[..]);
//!
//! // CTR mode can be used with streaming messages
//! let mut cipher = Aes128Ctr64LE::new(&key.into(), &iv.into());
//! for chunk in buf.chunks_mut(3) {
//!     cipher.apply_keystream(chunk);
//! }
//! assert_eq!(buf[..], plaintext[..]);
//!
//! // CTR mode supports seeking. The parameter is zero-based _bytes_ counter (not _blocks_).
//! cipher.seek(0u32);
//!
//! // encrypt/decrypt from buffer to buffer
//! // buffer length must be equal to input length
//! let mut buf1 = [0u8; 34];
//! cipher
//!     .apply_keystream_b2b(&plaintext, &mut buf1)
//!     .unwrap();
//! assert_eq!(buf1[..], ciphertext[..]);
//!
//! let mut buf2 = [0u8; 34];
//! cipher.seek(0u32);
//! cipher.apply_keystream_b2b(&buf1, &mut buf2).unwrap();
//! assert_eq!(buf2[..], plaintext[..]);
//! ```
//!
//! [1]: https://en.wikipedia.org/wiki/Block_cipher_mode_of_operation#CTR

#![no_std]
#![doc(
    html_logo_url = "https://raw.githubusercontent.com/RustCrypto/media/6ee8e381/logo.svg",
    html_favicon_url = "https://raw.githubusercontent.com/RustCrypto/media/6ee8e381/logo.svg"
)]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![warn(missing_docs, rust_2018_idioms)]

pub mod flavors;

mod backend;
mod ctr_core;

pub use cipher;
pub use flavors::CtrFlavor;

use cipher::StreamCipherCoreWrapper;
pub use ctr_core::CtrCore;

/// CTR mode with 128-bit big endian counter.
pub type Ctr128BE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr128BE>>;
/// CTR mode with 128-bit little endian counter.
pub type Ctr128LE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr128LE>>;
/// CTR mode with 64-bit big endian counter.
pub type Ctr64BE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr64BE>>;
/// CTR mode with 64-bit little endian counter.
pub type Ctr64LE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr64LE>>;
/// CTR mode with 32-bit big endian counter.
pub type Ctr32BE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr32BE>>;
/// CTR mode with 32-bit little endian counter.
pub type Ctr32LE<B> = StreamCipherCoreWrapper<CtrCore<B, flavors::Ctr32LE>>;
