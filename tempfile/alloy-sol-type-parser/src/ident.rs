use winnow::{
    error::{ErrMode, ParserError},
    stream::{AsBStr, Stream},
    ModalResult,
};

/// The regular expression for a Solidity identifier.
///
/// <https://docs.soliditylang.org/en/latest/grammar.html#a4.SolidityLexer.Identifier>
pub const IDENT_REGEX: &str = "[a-zA-Z$_][a-zA-Z0-9$_]*";

/// Returns `true` if the given character is valid at the start of a Solidity
/// identifier.
#[inline]
pub const fn is_id_start(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '_' | '$')
}

/// Returns `true` if the given character is valid in a Solidity identifier.
#[inline]
pub const fn is_id_continue(c: char) -> bool {
    matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '$')
}

/// Returns `true` if the given string is a valid Solidity identifier.
///
/// An identifier in Solidity has to start with a letter, a dollar-sign or
/// an underscore and may additionally contain numbers after the first
/// symbol.
///
/// Solidity reference:
/// <https://docs.soliditylang.org/en/latest/grammar.html#a4.SolidityLexer.Identifier>
pub const fn is_valid_identifier(s: &str) -> bool {
    // Note: valid idents can only contain ASCII characters, so we can
    // use the byte representation here.
    let [first, rest @ ..] = s.as_bytes() else {
        return false;
    };

    if !is_id_start(*first as char) {
        return false;
    }

    let mut i = 0;
    while i < rest.len() {
        if !is_id_continue(rest[i] as char) {
            return false;
        }
        i += 1;
    }

    true
}

/// Parses a Solidity identifier.
#[inline]
pub fn identifier<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    identifier_parser(input)
}

#[inline]
pub(crate) fn identifier_parser<'a, I>(input: &mut I) -> ModalResult<&'a str>
where
    I: Stream<Slice = &'a str> + AsBStr,
{
    // See note in `is_valid_identifier` above.
    // Use the faster `slice::Iter` instead of `str::Chars`.
    let mut chars = input.as_bstr().iter().map(|b| *b as char);

    let Some(true) = chars.next().map(is_id_start) else {
        return Err(ErrMode::from_input(input));
    };

    // 1 for the first character, we know it's ASCII
    let len = 1 + chars.take_while(|c| is_id_continue(*c)).count();
    Ok(input.next_slice(len))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_identifier() {
        ident_test("foo", Ok("foo"), "");
        ident_test("foo ", Ok("foo"), " ");
        ident_test("$foo", Ok("$foo"), "");
        ident_test("foo$", Ok("foo$"), "");
        ident_test("foo2$", Ok("foo2$"), "");
        ident_test("foo 2$", Ok("foo"), " 2$");
        ident_test("_foo 2$", Ok("_foo"), " 2$");

        ident_test("èfoo", Err(()), "èfoo");
        ident_test("fèoo", Ok("f"), "èoo");
        ident_test("foèo", Ok("fo"), "èo");
        ident_test("fooè", Ok("foo"), "è");

        ident_test("3foo", Err(()), "3foo");
        ident_test("f3oo", Ok("f3oo"), "");
        ident_test("fo3o", Ok("fo3o"), "");
        ident_test("foo3", Ok("foo3"), "");
    }

    #[track_caller]
    fn ident_test(mut input: &str, expected: Result<&str, ()>, output: &str) {
        assert_eq!(identifier(&mut input).map_err(drop), expected, "result mismatch");
        if let Ok(expected) = expected {
            assert!(is_valid_identifier(expected), "expected is not a valid ident");
        }
        assert_eq!(input, output, "output mismatch");
    }
}
