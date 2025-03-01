use crate::{abi::token::WordToken, Error, Result, SolType};
use alloc::borrow::Cow;

#[allow(unknown_lints, unnameable_types)]
mod sealed {
    pub trait Sealed {}
}
use sealed::Sealed;

/// A list of Solidity event topics.
///
/// This trait is implemented only on tuples of arity up to 4. The tuples must
/// contain only [`SolType`]s where the token is a [`WordToken`], and as such
/// it is sealed to prevent prevent incorrect downstream implementations.
///
/// See the [Solidity event ABI specification][solevent] for more details on how
/// events' topics are encoded.
///
/// [solevent]: https://docs.soliditylang.org/en/latest/abi-spec.html#events
///
/// # Implementer's Guide
///
/// It should not be necessary to implement this trait manually. Instead, use
/// the [`sol!`](crate::sol!) procedural macro to parse Solidity syntax into
/// types that implement this trait.
pub trait TopicList: SolType + Sealed {
    /// The number of topics.
    const COUNT: usize;

    /// Detokenize the topics into a tuple of rust types.
    ///
    /// This function accepts an iterator of `WordToken`.
    fn detokenize<I, D>(topics: I) -> Result<Self::RustType>
    where
        I: IntoIterator<Item = D>,
        D: Into<WordToken>;
}

macro_rules! impl_topic_list_tuples {
    ($($c:literal => $($lt:lifetime $t:ident),*;)+) => {$(
        impl<$($t,)*> Sealed for ($($t,)*) {}
        impl<$($lt,)* $($t: SolType<Token<$lt> = WordToken>,)*> TopicList for ($($t,)*) {
            const COUNT: usize = $c;

            fn detokenize<I, D>(topics: I) -> Result<Self::RustType>
            where
                I: IntoIterator<Item = D>,
                D: Into<WordToken>
            {
                let mut iter = topics.into_iter();
                Ok(($(
                    <$t>::detokenize(iter.next().ok_or_else(length_mismatch)?.into()),
                )*))
            }
        }
    )+};
}

impl Sealed for () {}
impl TopicList for () {
    const COUNT: usize = 0;

    #[inline]
    fn detokenize<I, D>(_: I) -> Result<Self::RustType>
    where
        I: IntoIterator<Item = D>,
        D: Into<WordToken>,
    {
        Ok(())
    }
}

impl_topic_list_tuples! {
    1 => 'a T;
    2 => 'a T, 'b U;
    3 => 'a T, 'b U, 'c V;
    4 => 'a T, 'b U, 'c V, 'd W;
}

#[cold]
const fn length_mismatch() -> Error {
    Error::Other(Cow::Borrowed("topic list length mismatch"))
}
