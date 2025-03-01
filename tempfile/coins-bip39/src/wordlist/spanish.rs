use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the Spanish language.
pub const RAW_SPANISH: &str = include_str!("./words/spanish.txt");

/// Spanish word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_SPANISH.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Spanish wordlist that implements the Wordlist trait.
pub struct Spanish;

impl Wordlist for Spanish {
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
        assert_eq!(Spanish::get(3), Ok("abierto"));
        assert_eq!(Spanish::get(2044), Ok("zona"));
        assert_eq!(Spanish::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Spanish::get_index("abierto"), Ok(3));
        assert_eq!(Spanish::get_index("zona"), Ok(2044));
        assert_eq!(
            Spanish::get_index("algunapalabraalazar"),
            Err(WordlistError::InvalidWord(
                "algunapalabraalazar".to_string()
            ))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Spanish::get_all().len(), 2048);
    }
}
