use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the Czech language.
pub const RAW_CZECH: &str = include_str!("./words/czech.txt");

/// Czech word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_CZECH.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Czech wordlist that implements the Wordlist trait.
pub struct Czech;

impl Wordlist for Czech {
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
        assert_eq!(Czech::get(3), Ok("agrese"));
        assert_eq!(Czech::get(2044), Ok("zvon"));
        assert_eq!(Czech::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Czech::get_index("agrese"), Ok(3));
        assert_eq!(Czech::get_index("zvon"), Ok(2044));
        assert_eq!(
            Czech::get_index("nějakénáhodnéslovo"),
            Err(WordlistError::InvalidWord("nějakénáhodnéslovo".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Czech::get_all().len(), 2048);
    }
}
