/// The English wordlist
#[cfg(feature = "english")]
pub mod english;
#[cfg(feature = "english")]
pub use self::english::English;

/// The Chinese (Simplified) wordlist
#[cfg(feature = "chinese-simplified")]
pub mod chinese_simplified;
#[cfg(feature = "chinese-simplified")]
pub use self::chinese_simplified::ChineseSimplified;

/// The Chinese (Traditional) wordlist
#[cfg(feature = "chinese-traditional")]
pub mod chinese_traditional;
#[cfg(feature = "chinese-traditional")]
pub use super::chinese_traditional::ChineseTraditional;

/// The Czech wordlist
#[cfg(feature = "czech")]
pub mod czech;
#[cfg(feature = "czech")]
pub use super::czech::Czech;

/// The French wordlist
#[cfg(feature = "french")]
pub mod french;
#[cfg(feature = "french")]
pub use super::french::French;

/// The Italian wordlist
#[cfg(feature = "italian")]
pub mod italian;
#[cfg(feature = "italian")]
pub use super::italian::Italian;

/// The Japanese wordlist
#[cfg(feature = "japanese")]
pub mod japanese;
#[cfg(feature = "japanese")]
pub use super::japanese::Japanese;

/// The Korean wordlist
#[cfg(feature = "korean")]
pub mod korean;
#[cfg(feature = "korean")]
pub use super::korean::Korean;

/// The Portuguese wordlist
#[cfg(feature = "portuguese")]
pub mod portuguese;
#[cfg(feature = "portuguese")]
pub use super::portuguese::Portuguese;

/// The Spanish wordlist
#[cfg(feature = "spanish")]
pub mod spanish;
#[cfg(feature = "spanish")]
pub use super::spanish::Spanish;

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
/// The error type returned while interacting with wordists.
pub enum WordlistError {
    /// Describes the error when the wordlist is queried at an invalid index.
    #[error("the index `{0}` is invalid")]
    InvalidIndex(usize),
    /// Describes the error when the wordlist does not contain the queried word.
    #[error("the word `{0}` is invalid")]
    InvalidWord(String),
}

/// The Wordlist trait that every language's wordlist must implement.
pub trait Wordlist {
    /// Returns the word list as a string.
    ///
    /// Implementor's note: this MUST be sorted
    fn get_all() -> &'static [&'static str];

    /// Returns the word of a given index from the word list.
    fn get(index: usize) -> Result<&'static str, WordlistError> {
        Self::get_all()
            .get(index)
            .map(std::ops::Deref::deref)
            .ok_or(crate::WordlistError::InvalidIndex(index))
    }

    /// Returns the index of a given word from the word list.
    fn get_index(word: &str) -> Result<usize, WordlistError> {
        Self::get_all()
            .iter()
            .position(|&x| x == word)
            .ok_or(crate::WordlistError::InvalidWord(word.to_string()))
    }
}
