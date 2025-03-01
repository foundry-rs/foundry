use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the French language.
pub const RAW_FRENCH: &str = include_str!("./words/french.txt");

/// French word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_FRENCH.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The French wordlist that implements the Wordlist trait.
pub struct French;

impl Wordlist for French {
    fn get_all() -> &'static [&'static str] {
        PARSED.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::WordlistError;

    #[test]
    fn test_get() {
        assert_eq!(French::get(3), Ok("abeille"));
        assert_eq!(French::get(2044), Ok("zèbre"));
        assert_eq!(French::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(French::get_index("abeille"), Ok(3));
        assert_eq!(French::get_index("zèbre"), Ok(2044));
        assert_eq!(
            French::get_index("unmotauhasard"),
            Err(WordlistError::InvalidWord("unmotauhasard".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(French::get_all().len(), 2048);
    }
}
