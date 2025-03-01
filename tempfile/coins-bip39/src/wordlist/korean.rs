use crate::{Wordlist, WordlistError};
use once_cell::sync::Lazy;

/// The list of words as supported in the Korean language.
pub const RAW_KOREAN: &str = include_str!("./words/korean.txt");

/// Korean word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_KOREAN.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Korean wordlist that implements the Wordlist trait.
pub struct Korean;

impl Wordlist for Korean {
    fn get_all() -> &'static [&'static str] {
        PARSED.as_slice()
    }
    /// Returns the index of a given word from the word list.
    fn get_index(word: &str) -> Result<usize, WordlistError> {
        Self::get_all()
            .binary_search(&word)
            .map_err(|_| crate::WordlistError::InvalidWord(word.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::WordlistError;

    #[test]
    fn test_get() {
        assert_eq!(Korean::get(3), Ok("가능"));
        assert_eq!(Korean::get(2044), Ok("희망"));
        assert_eq!(Korean::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Korean::get_index("가능"), Ok(3));
        assert_eq!(Korean::get_index("희망"), Ok(2044));
        assert_eq!(
            Korean::get_index("임의의단어"),
            Err(WordlistError::InvalidWord("임의의단어".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Korean::get_all().len(), 2048);
    }
}
