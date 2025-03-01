use crate::{Wordlist, WordlistError};
use once_cell::sync::Lazy;

/// The list of words as supported in the Portuguese language.
pub const RAW_PORTUGUESE: &str = include_str!("./words/portuguese.txt");

/// Portuguese word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_PORTUGUESE.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Portuguese wordlist that implements the Wordlist trait.
pub struct Portuguese;

impl Wordlist for Portuguese {
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
        assert_eq!(Portuguese::get(3), Ok("abater"));
        assert_eq!(Portuguese::get(2044), Ok("zelador"));
        assert_eq!(
            Portuguese::get(2048),
            Err(WordlistError::InvalidIndex(2048))
        );
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Portuguese::get_index("abater"), Ok(3));
        assert_eq!(Portuguese::get_index("zelador"), Ok(2044));
        assert_eq!(
            Portuguese::get_index("algumapalavraaleatória"),
            Err(WordlistError::InvalidWord(
                "algumapalavraaleatória".to_string()
            ))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Portuguese::get_all().len(), 2048);
    }
}
