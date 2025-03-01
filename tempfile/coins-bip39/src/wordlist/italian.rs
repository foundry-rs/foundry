use crate::{Wordlist, WordlistError};
use once_cell::sync::Lazy;

/// The list of words as supported in the Italian language.
pub const RAW_ITALIAN: &str = include_str!("./words/italian.txt");

/// Italian word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_ITALIAN.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Italian wordlist that implements the Wordlist trait.
pub struct Italian;

impl Wordlist for Italian {
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
        assert_eq!(Italian::get(3), Ok("abete"));
        assert_eq!(Italian::get(2044), Ok("zucchero"));
        assert_eq!(Italian::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Italian::get_index("abete"), Ok(3));
        assert_eq!(Italian::get_index("zucchero"), Ok(2044));
        assert_eq!(
            Italian::get_index("qualcheparolaacaso"),
            Err(WordlistError::InvalidWord("qualcheparolaacaso".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Italian::get_all().len(), 2048);
    }
}
