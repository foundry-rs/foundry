use std::cmp::Ordering;

/// Compares strings by treating internal integers as atomic units.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    Iterator::cmp(Tokenizer { input: a }, Tokenizer { input: b })
}

#[inline]
fn cmp_int(mut a: &str, mut b: &str) -> Ordering {
    a = a.trim_start_matches('0');
    b = b.trim_start_matches('0');

    // Compare to 0.
    match (a.is_empty(), b.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
        _ => {}
    }

    // Compare length.
    match a.len().cmp(&b.len()) {
        Ordering::Equal => {}
        ord => return ord,
    }

    // Compare digits.
    a.cmp(b)
}

#[derive(PartialEq, Eq)]
#[cfg_attr(test, derive(Debug))]
struct Token<'a> {
    is_int: bool,
    text: &'a str,
}

impl PartialOrd for Token<'_> {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Token<'_> {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        if self.is_int && other.is_int {
            cmp_int(self.text, other.text)
        } else {
            self.text.cmp(other.text)
        }
    }
}

/// Lexes a string into "tokens".
struct Tokenizer<'a> {
    /// The remaining characters to process.
    input: &'a str,
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = Token<'a>;

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        let mut bytes = self.input.bytes();
        let is_int = bytes.next()?.is_ascii_digit();

        let mut kind_len = 1;
        for ch in bytes {
            // Stop on character kind change.
            if ch.is_ascii_digit() != is_int {
                break;
            }

            kind_len += 1;
        }

        unsafe {
            let text = self.input.get_unchecked(..kind_len);
            self.input = self.input.get_unchecked(kind_len..);

            Some(Token { is_int, text })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn test_sort(list: &[&str], cmp: fn(&str, &str) -> Ordering) {
        let mut copy = list.to_vec();
        copy.sort_by(|a, b| cmp(a, b));
        assert_eq!(list, copy);
    }

    #[test]
    fn natural_cmp() {
        #[track_caller]
        fn test(list: &[&str]) {
            test_sort(list, super::natural_cmp);
        }

        test(&["A<4>", "A<8>", "A<16>", "A<32>", "A<64>"]);
    }

    #[test]
    fn cmp_int() {
        #[track_caller]
        fn test(list: &[&str]) {
            test_sort(list, super::cmp_int);
        }

        test(&["4", "8", "16", "32", "64"]);
        test(&["4", "08"]);
        test(&["0", "00"]);
    }

    #[test]
    fn tokenize() {
        #[track_caller]
        fn test(s: &str, expected: &[Token]) {
            let tokens: Vec<Token> = Tokenizer { input: s }.collect();
            assert_eq!(tokens, expected);
        }

        test(
            "A<4>",
            &[
                Token { text: "A<", is_int: false },
                Token { text: "4", is_int: true },
                Token { text: ">", is_int: false },
            ],
        );
    }
}
