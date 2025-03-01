use crate::Wordlist;
use once_cell::sync::Lazy;

/// The list of words as supported in the Chinese (Traditional) language.
pub const RAW_CHINESE_TRADITIONAL: &str = include_str!("./words/chinese_traditional.txt");

/// ChineseTraditional word list, split into words
pub static PARSED: Lazy<Vec<&'static str>> =
    Lazy::new(|| RAW_CHINESE_TRADITIONAL.lines().collect());

#[derive(Clone, Debug, PartialEq, Eq, Copy)]
/// The ChineseTraditional wordlist that implements the Wordlist trait.
pub struct ChineseTraditional;

impl Wordlist for ChineseTraditional {
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
        assert_eq!(ChineseTraditional::get(3), Ok("在"));
        assert_eq!(ChineseTraditional::get(2044), Ok("韋"));
        assert_eq!(
            ChineseTraditional::get(2048),
            Err(WordlistError::InvalidIndex(2048))
        );
    }

    #[test]
    fn test_get_index() {
        assert_eq!(ChineseTraditional::get_index("在"), Ok(3));
        assert_eq!(ChineseTraditional::get_index("韋"), Ok(2044));
        assert_eq!(
            ChineseTraditional::get_index("龜"),
            Err(WordlistError::InvalidWord("龜".to_string()))
        );
    }

    #[test]
    fn test_get_all() {
        assert_eq!(ChineseTraditional::get_all().len(), 2048);
    }
}
