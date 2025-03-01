use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the Japanese language.
pub const RAW_JAPANESE: &str = include_str!("./words/japanese.txt");

/// Japanese word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_JAPANESE.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The Japanese wordlist that implements the Wordlist trait.
pub struct Japanese;

impl Wordlist for Japanese {
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
        assert_eq!(Japanese::get(3), Ok("あおぞら"));
        assert_eq!(Japanese::get(2044), Ok("わじまし"));
        assert_eq!(Japanese::get(2048), Err(WordlistError::InvalidIndex(2048)));
    }

    #[test]
    fn test_get_index() {
        assert_eq!(Japanese::get_index("あおぞら"), Ok(3));
        assert_eq!(Japanese::get_index("わじまし"), Ok(2044));
        assert_eq!(
            Japanese::get_index("ランダムな単語"),
            Err(WordlistError::InvalidWord("ランダムな単語".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(Japanese::get_all().len(), 2048);
    }
}
