use crate::{Decoder, DynSolValue, Error, Result, Word};
use alloc::{borrow::Cow, boxed::Box, vec::Vec};
use alloy_primitives::try_vec;
use alloy_sol_types::abi::token::{PackedSeqToken, Token, WordToken};

/// A dynamic token.
///
/// Equivalent to an enum over all types implementing [`Token`].
// NOTE: do not derive `Hash` for this type. The derived version is not
// compatible with the current `PartialEq` implementation. If manually
// implementing `Hash`, ignore the `template` prop in the `DynSeq` variant
#[derive(Clone, Debug)]
pub enum DynToken<'a> {
    /// A single word.
    Word(Word),
    /// A Fixed Sequence.
    FixedSeq(Cow<'a, [DynToken<'a>]>, usize),
    /// A dynamic-length sequence.
    DynSeq {
        /// The contents of the dynamic sequence.
        contents: Cow<'a, [DynToken<'a>]>,
        /// The type template of the dynamic sequence.
        /// This is used only when decoding. It indicates what the token type
        /// of the sequence is. During tokenization of data, the type of the
        /// contents is known, so this is not needed.
        #[doc(hidden)]
        template: Option<Box<DynToken<'a>>>,
    },
    /// A packed sequence (string or bytes).
    PackedSeq(&'a [u8]),
}

impl<T: Into<Word>> From<T> for DynToken<'_> {
    #[inline]
    fn from(value: T) -> Self {
        Self::Word(value.into())
    }
}

impl PartialEq<DynToken<'_>> for DynToken<'_> {
    #[inline]
    fn eq(&self, other: &DynToken<'_>) -> bool {
        match (self, other) {
            (Self::Word(l0), DynToken::Word(r0)) => l0 == r0,
            (Self::FixedSeq(l0, l1), DynToken::FixedSeq(r0, r1)) => l0 == r0 && l1 == r1,
            (
                Self::DynSeq { contents: l_contents, .. },
                DynToken::DynSeq { contents: r_contents, .. },
            ) => l_contents == r_contents,
            (Self::PackedSeq(l0), DynToken::PackedSeq(r0)) => l0 == r0,
            _ => false,
        }
    }
}

impl Eq for DynToken<'_> {}

impl<'a> DynToken<'a> {
    /// Calculate the minimum number of words required to encode this token.
    pub fn minimum_words(&self) -> usize {
        match self {
            DynToken::Word(_) => 1,
            DynToken::PackedSeq(_) => 1,
            DynToken::FixedSeq(contents, _) => {
                contents.iter().map(Self::minimum_words).sum::<usize>()
            }
            DynToken::DynSeq { .. } => 1,
        }
    }

    /// Instantiate a DynToken from a fixed sequence of values.
    #[inline]
    pub fn from_fixed_seq(seq: &'a [DynSolValue]) -> Self {
        let tokens = seq.iter().map(DynSolValue::tokenize).collect();
        Self::FixedSeq(Cow::Owned(tokens), seq.len())
    }

    /// Instantiate a DynToken from a dynamic sequence of values.
    #[inline]
    pub fn from_dyn_seq(seq: &'a [DynSolValue]) -> Self {
        let tokens = seq.iter().map(DynSolValue::tokenize).collect();
        Self::DynSeq { contents: Cow::Owned(tokens), template: None }
    }

    /// Attempt to cast to a word.
    #[inline]
    pub const fn as_word(&self) -> Option<Word> {
        match self {
            Self::Word(word) => Some(*word),
            _ => None,
        }
    }

    /// Fallible cast into a fixed sequence.
    #[inline]
    pub fn as_fixed_seq(&self) -> Option<(&[Self], usize)> {
        match self {
            Self::FixedSeq(tokens, size) => Some((tokens, *size)),
            _ => None,
        }
    }

    /// Fallible cast into a dynamic sequence.
    #[inline]
    pub fn as_dynamic_seq(&self) -> Option<&[Self]> {
        match self {
            Self::DynSeq { contents, .. } => Some(contents),
            _ => None,
        }
    }

    /// Fallible cast into a sequence, dynamic or fixed-size
    #[inline]
    pub fn as_token_seq(&self) -> Option<&[Self]> {
        match self {
            Self::FixedSeq(contents, _) | Self::DynSeq { contents, .. } => Some(contents),
            _ => None,
        }
    }

    /// Fallible cast into a packed sequence.
    #[inline]
    pub const fn as_packed_seq(&self) -> Option<&[u8]> {
        match self {
            Self::PackedSeq(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// True if the type is dynamic, else false.
    #[inline]
    pub fn is_dynamic(&self) -> bool {
        match self {
            Self::Word(_) => false,
            Self::FixedSeq(inner, _) => inner.iter().any(Self::is_dynamic),
            Self::DynSeq { .. } | Self::PackedSeq(_) => true,
        }
    }

    /// Decodes from a decoder, populating the structure with the decoded data.
    #[inline]
    pub(crate) fn decode_populate(&mut self, dec: &mut Decoder<'a>) -> Result<()> {
        match self {
            Self::Word(w) => *w = WordToken::decode_from(dec)?.0,
            Self::FixedSeq(..) => {
                let dynamic = self.is_dynamic();
                let mut child = if dynamic { dec.take_indirection() } else { dec.raw_child() }?;

                self.decode_sequence_populate(&mut child)?;

                if !dynamic {
                    dec.take_offset_from(&child);
                }
            }
            Self::DynSeq { contents, template } => {
                let mut child = dec.take_indirection()?;
                let size = child.take_offset()?;
                if size == 0 {
                    // should already be empty from `empty_dyn_token`
                    debug_assert!(contents.is_empty());
                    return Ok(());
                }

                // This expect is safe because this is only invoked after
                // `empty_dyn_token()` which always sets template
                let template = template.take().expect("no template for dynamic sequence");

                // This appears to be an unclarity in the Solidity spec. The
                // spec specifies that offsets are relative to the beginning of
                // `enc(X)`. But known-good test vectors have it relative to the
                // word AFTER the array size
                let mut child = child.raw_child()?;

                // Check that the decoder contains enough words to decode the
                // sequence. Each item in the sequence is at least one word, so
                // the remaining words must be at least the size of the sequence
                if child.remaining_words() < template.minimum_words() * size {
                    return Err(alloy_sol_types::Error::Overrun.into());
                }

                let mut new_tokens = if size == 1 {
                    // re-use the box allocation
                    unsafe { Vec::from_raw_parts(Box::into_raw(template), 1, 1) }
                } else {
                    try_vec![*template; size]?
                };

                for t in &mut new_tokens {
                    t.decode_populate(&mut child)?;
                }

                *contents = new_tokens.into();
            }
            Self::PackedSeq(buf) => *buf = PackedSeqToken::decode_from(dec)?.0,
        }
        Ok(())
    }

    /// Decode a sequence from the decoder, populating the data by consuming
    /// decoder words.
    #[inline]
    pub(crate) fn decode_sequence_populate(&mut self, dec: &mut Decoder<'a>) -> Result<()> {
        match self {
            Self::FixedSeq(buf, size) => {
                buf.to_mut().iter_mut().take(*size).try_for_each(|item| item.decode_populate(dec))
            }
            Self::DynSeq { .. } => self.decode_populate(dec),
            _ => Err(Error::custom("Called decode_sequence_populate on non-sequence token")),
        }
    }

    /// Decode a single item of this type, as a sequence of length 1.
    #[inline]
    pub(crate) fn decode_single_populate(&mut self, dec: &mut Decoder<'a>) -> Result<()> {
        // This is what
        // `Self::FixedSeq(vec![self.clone()], 1).decode_populate()`
        // would do, so we skip the allocation.
        self.decode_populate(dec)
    }
}
