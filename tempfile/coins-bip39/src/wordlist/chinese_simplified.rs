use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the Chinese (Simplified) language.
pub const RAW_CHINESE_SIMPLIFIED: &str = include_str!("./words/chinese_simplified.txt");

/// ChineseSimplified word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> = Lazy::new(|| RAW_CHINESE_SIMPLIFIED.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The ChineseSimplified wordlist that implements the Wordlist trait.
pub struct ChineseSimplified;

impl Wordlist for ChineseSimplified {
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
        assert_eq!(ChineseSimplified::get(3), Ok("在"));
        assert_eq!(ChineseSimplified::get(2044), Ok("韦"));
        assert_eq!(
            ChineseSimplified::get(2048),
            Err(WordlistError::InvalidIndex(2048))
        );
    }

    #[test]
    fn test_get_index() {
        assert_eq!(ChineseSimplified::get_index("在"), Ok(3));
        assert_eq!(ChineseSimplified::get_index("韦"), Ok(2044));
        assert_eq!(
            ChineseSimplified::get_index("龟"),
            Err(WordlistError::InvalidWord("龟".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(ChineseSimplified::get_all().len(), 2048);
    }
}
