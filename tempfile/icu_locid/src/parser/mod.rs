// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

pub mod errors;
mod langid;
mod locale;

pub use errors::ParserError;
pub use langid::{
    parse_language_identifier, parse_language_identifier_from_iter,
    parse_language_identifier_with_single_variant,
    parse_locale_with_single_variant_single_keyword_unicode_extension_from_iter, ParserMode,
};

pub use locale::{
    parse_locale, parse_locale_with_single_variant_single_keyword_unicode_keyword_extension,
};

#[inline]
const fn is_separator(slice: &[u8], idx: usize) -> bool {
    #[allow(clippy::indexing_slicing)]
    let b = slice[idx];
    b == b'-' || b == b'_'
}

const fn get_current_subtag(slice: &[u8], idx: usize) -> (usize, usize) {
    debug_assert!(idx < slice.len());

    // This function is called only on the idx == 0 or on a separator.
    let (start, mut end) = if is_separator(slice, idx) {
        // If it's a separator, set the start to idx+1 and advance the idx to the next char.
        (idx + 1, idx + 1)
    } else {
        // If it's idx=0, start is 0 and end is set to 1
        debug_assert!(idx == 0);
        (0, 1)
    };

    while end < slice.len() && !is_separator(slice, end) {
        // Advance until we reach end of slice or a separator.
        end += 1;
    }
    // Notice: this slice may be empty (start == end) for cases like `"en-"` or `"en--US"`
    (start, end)
}

// `SubtagIterator` is a helper iterator for [`LanguageIdentifier`] and [`Locale`] parsing.
//
// It is quite extraordinary due to focus on performance and Rust limitations for `const`
// functions.
//
// The iterator is eager and fallible allowing it to reject invalid slices such as `"-"`, `"-en"`,
// `"en-"` etc.
//
// The iterator provides methods available for static users - `next_manual` and `peek_manual`,
// as well as typical `Peekable` iterator APIs - `next` and `peek`.
//
// All methods return an `Option` of a `Result`.
#[derive(Copy, Clone, Debug)]
pub struct SubtagIterator<'a> {
    pub slice: &'a [u8],
    done: bool,
    // done + subtag is faster than Option<(usize, usize)>
    // at the time of writing.
    subtag: (usize, usize),
}

impl<'a> SubtagIterator<'a> {
    pub const fn new(slice: &'a [u8]) -> Self {
        let subtag = if slice.is_empty() || is_separator(slice, 0) {
            // This returns (0, 0) which returns Some(b"") for slices like `"-en"` or `"-"`
            (0, 0)
        } else {
            get_current_subtag(slice, 0)
        };
        Self {
            slice,
            done: false,
            subtag,
        }
    }

    pub const fn next_manual(mut self) -> (Self, Option<(usize, usize)>) {
        if self.done {
            return (self, None);
        }
        let result = self.subtag;
        if result.1 < self.slice.len() {
            self.subtag = get_current_subtag(self.slice, result.1);
        } else {
            self.done = true;
        }
        (self, Some(result))
    }

    pub const fn peek_manual(&self) -> Option<(usize, usize)> {
        if self.done {
            return None;
        }
        Some(self.subtag)
    }

    pub fn peek(&self) -> Option<&'a [u8]> {
        #[allow(clippy::indexing_slicing)] // peek_manual returns valid indices
        self.peek_manual().map(|(s, e)| &self.slice[s..e])
    }
}

impl<'a> Iterator for SubtagIterator<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        let (s, res) = self.next_manual();
        *self = s;
        #[allow(clippy::indexing_slicing)] // next_manual returns valid indices
        res.map(|(s, e)| &self.slice[s..e])
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn slice_to_str(input: &[u8]) -> &str {
        std::str::from_utf8(input).unwrap()
    }

    #[test]
    fn subtag_iterator_peek_test() {
        let slice = "de_at-u-ca-foobar";
        let mut si = SubtagIterator::new(slice.as_bytes());

        assert_eq!(si.peek().map(slice_to_str), Some("de"));
        assert_eq!(si.peek().map(slice_to_str), Some("de"));
        assert_eq!(si.next().map(slice_to_str), Some("de"));

        assert_eq!(si.peek().map(slice_to_str), Some("at"));
        assert_eq!(si.peek().map(slice_to_str), Some("at"));
        assert_eq!(si.next().map(slice_to_str), Some("at"));
    }

    #[test]
    fn subtag_iterator_test() {
        let slice = "";
        let mut si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.next().map(slice_to_str), Some(""));

        let slice = "-";
        let mut si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.next().map(slice_to_str), Some(""));

        let slice = "-en";
        let mut si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next().map(slice_to_str), Some("en"));
        assert_eq!(si.next(), None);

        let slice = "en";
        let si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.map(slice_to_str).collect::<Vec<_>>(), vec!["en",]);

        let slice = "en-";
        let si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.map(slice_to_str).collect::<Vec<_>>(), vec!["en", "",]);

        let slice = "--";
        let mut si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next(), None);

        let slice = "-en-";
        let mut si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next().map(slice_to_str), Some("en"));
        assert_eq!(si.next().map(slice_to_str), Some(""));
        assert_eq!(si.next(), None);

        let slice = "de_at-u-ca-foobar";
        let si = SubtagIterator::new(slice.as_bytes());
        assert_eq!(
            si.map(slice_to_str).collect::<Vec<_>>(),
            vec!["de", "at", "u", "ca", "foobar",]
        );
    }

    #[test]
    fn get_current_subtag_test() {
        let slice = "-";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (1, 1));

        let slice = "-en";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (1, 3));

        let slice = "-en-";
        let current = get_current_subtag(slice.as_bytes(), 3);
        assert_eq!(current, (4, 4));

        let slice = "en-";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (0, 2));

        let current = get_current_subtag(slice.as_bytes(), 2);
        assert_eq!(current, (3, 3));

        let slice = "en--US";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (0, 2));

        let current = get_current_subtag(slice.as_bytes(), 2);
        assert_eq!(current, (3, 3));

        let current = get_current_subtag(slice.as_bytes(), 3);
        assert_eq!(current, (4, 6));

        let slice = "--";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (1, 1));

        let current = get_current_subtag(slice.as_bytes(), 1);
        assert_eq!(current, (2, 2));

        let slice = "-";
        let current = get_current_subtag(slice.as_bytes(), 0);
        assert_eq!(current, (1, 1));
    }
}
