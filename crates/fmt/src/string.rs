//! Helpers for dealing with quoted strings

/// The state of a character in a string with quotable components
/// This is a simplified version of the
/// [actual parser](https://docs.soliditylang.org/en/v0.8.15/grammar.html#a4.SolidityLexer.EscapeSequence)
/// as we don't care about hex or other character meanings
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QuoteState {
    /// Not currently in quoted string
    #[default]
    None,
    /// The opening character of a quoted string
    Opening(char),
    /// A character in a quoted string
    String(char),
    /// The `\` in an escape sequence `"\n"`
    Escaping(char),
    /// The escaped character e.g. `n` in `"\n"`
    Escaped(char),
    /// The closing character
    Closing(char),
}

/// An iterator over characters and indices in a string slice with information about quoted string
/// states
pub struct QuoteStateCharIndices<'a> {
    iter: std::str::CharIndices<'a>,
    state: QuoteState,
}

impl<'a> QuoteStateCharIndices<'a> {
    fn new(string: &'a str) -> Self {
        Self { iter: string.char_indices(), state: QuoteState::None }
    }
    pub fn with_state(mut self, state: QuoteState) -> Self {
        self.state = state;
        self
    }
}

impl<'a> Iterator for QuoteStateCharIndices<'a> {
    type Item = (QuoteState, usize, char);
    fn next(&mut self) -> Option<Self::Item> {
        let (idx, ch) = self.iter.next()?;
        match self.state {
            QuoteState::None | QuoteState::Closing(_) => {
                if ch == '\'' || ch == '"' {
                    self.state = QuoteState::Opening(ch);
                } else {
                    self.state = QuoteState::None
                }
            }
            QuoteState::String(quote) | QuoteState::Opening(quote) | QuoteState::Escaped(quote) => {
                if ch == quote {
                    self.state = QuoteState::Closing(quote)
                } else if ch == '\\' {
                    self.state = QuoteState::Escaping(quote)
                } else {
                    self.state = QuoteState::String(quote)
                }
            }
            QuoteState::Escaping(quote) => self.state = QuoteState::Escaped(quote),
        }
        Some((self.state, idx, ch))
    }
}

/// An iterator over the indices of quoted string locations
pub struct QuotedRanges<'a>(QuoteStateCharIndices<'a>);

impl<'a> QuotedRanges<'a> {
    pub fn with_state(mut self, state: QuoteState) -> Self {
        self.0 = self.0.with_state(state);
        self
    }
}

impl<'a> Iterator for QuotedRanges<'a> {
    type Item = (char, usize, usize);
    fn next(&mut self) -> Option<Self::Item> {
        let (quote, start) = loop {
            let (state, idx, _) = self.0.next()?;
            match state {
                QuoteState::Opening(quote) |
                QuoteState::Escaping(quote) |
                QuoteState::Escaped(quote) |
                QuoteState::String(quote) => break (quote, idx),
                QuoteState::Closing(quote) => return Some((quote, idx, idx)),
                QuoteState::None => {}
            }
        };
        for (state, idx, _) in self.0.by_ref() {
            if matches!(state, QuoteState::Closing(_)) {
                return Some((quote, start, idx))
            }
        }
        None
    }
}

/// Helpers for iterating over quoted strings
pub trait QuotedStringExt {
    /// Returns an iterator of characters, indices and their quoted string state.
    fn quote_state_char_indices(&self) -> QuoteStateCharIndices<'_>;

    /// Returns an iterator of quoted string ranges.
    fn quoted_ranges(&self) -> QuotedRanges<'_> {
        QuotedRanges(self.quote_state_char_indices())
    }

    /// Check to see if a string is quoted. This will return true if the first character
    /// is a quote and the last character is a quote with no non-quoted sections in between.
    fn is_quoted(&self) -> bool {
        let mut iter = self.quote_state_char_indices();
        if !matches!(iter.next(), Some((QuoteState::Opening(_), _, _))) {
            return false
        }
        while let Some((state, _, _)) = iter.next() {
            if matches!(state, QuoteState::Closing(_)) {
                return iter.next().is_none()
            }
        }
        false
    }
}

impl<T> QuotedStringExt for T
where
    T: AsRef<str>,
{
    fn quote_state_char_indices(&self) -> QuoteStateCharIndices<'_> {
        QuoteStateCharIndices::new(self.as_ref())
    }
}

impl QuotedStringExt for str {
    fn quote_state_char_indices(&self) -> QuoteStateCharIndices<'_> {
        QuoteStateCharIndices::new(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn quote_state_char_indices() {
        assert_eq!(
            r#"a'a"\'\"\n\\'a"#.quote_state_char_indices().collect::<Vec<_>>(),
            vec![
                (QuoteState::None, 0, 'a'),
                (QuoteState::Opening('\''), 1, '\''),
                (QuoteState::String('\''), 2, 'a'),
                (QuoteState::String('\''), 3, '"'),
                (QuoteState::Escaping('\''), 4, '\\'),
                (QuoteState::Escaped('\''), 5, '\''),
                (QuoteState::Escaping('\''), 6, '\\'),
                (QuoteState::Escaped('\''), 7, '"'),
                (QuoteState::Escaping('\''), 8, '\\'),
                (QuoteState::Escaped('\''), 9, 'n'),
                (QuoteState::Escaping('\''), 10, '\\'),
                (QuoteState::Escaped('\''), 11, '\\'),
                (QuoteState::Closing('\''), 12, '\''),
                (QuoteState::None, 13, 'a'),
            ]
        );
    }

    #[test]
    fn quoted_ranges() {
        let string = r#"testing "double quoted" and 'single quoted' strings"#;
        assert_eq!(
            string
                .quoted_ranges()
                .map(|(quote, start, end)| (quote, &string[start..=end]))
                .collect::<Vec<_>>(),
            vec![('"', r#""double quoted""#), ('\'', "'single quoted'")]
        );
    }
}
