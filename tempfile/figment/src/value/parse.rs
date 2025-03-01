use pear::{parse_error, parsers::*};
use pear::combinators::*;
use pear::macros::{parse, parser, switch};
use pear::input::{Pear, Text};

use crate::value::{Value, Dict, escape::escape};

type Input<'a> = Pear<Text<'a>>;
type Result<'a, T> = pear::input::Result<T, Input<'a>>;

#[inline(always)]
fn is_whitespace(&byte: &char) -> bool {
    byte.is_ascii_whitespace()
}

#[inline(always)]
fn is_not_separator(&byte: &char) -> bool {
    !matches!(byte, ',' | '{' | '}' | '[' | ']')
}

// TODO: Be more permissive here?
#[inline(always)]
fn is_ident_char(&byte: &char) -> bool {
    byte.is_ascii_alphanumeric() || byte == '_' || byte == '-'
}

#[parser]
fn string<'a>(input: &mut Input<'a>) -> Result<'a, String> {
    let mut is_escaped = false;
    let str_char = |&c: &char| -> bool {
        if is_escaped { is_escaped = false; return true; }
        if c == '\\' { is_escaped = true; return true; }
        c != '"'
    };

    let inner = (eat('"')?, take_while(str_char)?, eat('"')?).1;
    match escape(inner) {
        Ok(string) => string.into_owned(),
        Err(e) => parse_error!("invalid string: {}", e)?,
    }
}

#[parser]
fn key<'a>(input: &mut Input<'a>) -> Result<'a, String> {
    switch! {
        peek('"') => Ok(string()?),
        _ => Ok(take_some_while(is_ident_char)?.to_string())
    }
}

#[parser]
fn key_value<'a>(input: &mut Input<'a>) -> Result<'a, (String, Value)> {
    let key = (surrounded(key, is_whitespace)?, eat('=')?).0;
    (key, surrounded(value, is_whitespace)?)
}

#[parser]
fn array<'a>(input: &mut Input<'a>) -> Result<'a, Vec<Value>> {
    Ok(delimited_collect('[', value, ',', ']')?)
}

#[parser]
fn dict<'a>(input: &mut Input<'a>) -> Result<'a, Dict> {
    Ok(delimited_collect('{', key_value, ',', '}')?)
}

#[parser]
fn value<'a>(input: &mut Input<'a>) -> Result<'a, Value> {
    skip_while(is_whitespace)?;
    let val = switch! {
        eat_slice("true") => Value::from(true),
        eat_slice("false") => Value::from(false),
        peek('{') => Value::from(dict()?),
        peek('[') => Value::from(array()?),
        peek('"') => Value::from(string()?),
        peek('\'') => Value::from((eat('\'')?, eat_any()?, eat('\'')?).1),
        _ => {
            let value = take_while(is_not_separator)?.trim();
            if value.contains('.') {
                if let Ok(float) = value.parse::<f64>() {
                    return Ok(Value::from(float));
                }
            }

            if let Ok(int) = value.parse::<usize>() {
                Value::from(int)
            } else if let Ok(int) = value.parse::<isize>() {
                Value::from(int)
            } else {
                Value::from(value.to_string())
            }
        }
    };

    skip_while(is_whitespace)?;
    val
}

impl std::str::FromStr for Value {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, std::convert::Infallible> {
        Ok(parse!(value: Text::from(s))
            .unwrap_or_else(|_| Value::from(s.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::map;

    macro_rules! assert_parse_eq {
        ($string:expr => $expected:expr) => ({
            let expected = Value::from($expected);
            let actual: Value = $string.parse().unwrap();
            assert_eq!(actual, expected, "got {:?}, expected {:?}", actual, expected);
        });

        ($($string:expr => $expected:expr $(,)?)*) => (
            $(assert_parse_eq!($string => $expected);)*
        )
    }

    #[test]
    #[allow(clippy::approx_constant)] // false positive: using the PI constant would be wrong here
    fn check_simple_values_parse() {
        assert_parse_eq! {
            "true" => true,
            "false" => false,
            "\"false\"" => "false",
            "\"true\"" => "true",
            "  false" => false,
            "  false  " => false,
            "true  " => true,
            "1" => 1u8,
            " 0" => 0u8,
            " -0" => 0i8,
            " -2" => -2,
            " 123 " => 123u8,
            "\"a\"" => "a",
            "a " => "a",
            "   a " => "a",
            "\" a\"" => " a",
            "\"a  \"" => "a  ",
            "\" a  \"" => " a  ",
            "1.2" => 1.2,
            "  1.2" => 1.2,
            "3.14159" => 3.14159,
            "\"\\t\"" => "\t",
            "\"abc\\td\"" => "abc\td",
            "\"abc\\td\\n\"" => "abc\td\n",
            "\"abc\\td\\n\\n\"" => "abc\td\n\n",
            "\"abc\\td\"" => "abc\td",
            "\"\\\"\"" => "\"",
            "\"\\n\\f\\b\\\\\\r\\\"\"" => "\n\u{c}\u{8}\\\r\"",
            "\"\\\"hi\\\"\"" => "\"hi\"",
            "\"hi\\u1234there\"" => "hi\u{1234}there",
            "\"\\\\\"" => "\\",
            // unterminated strings pass through as themselves
            "\"\\\"" => "\"\\\"",
        }
    }

    #[test]
    fn check_compund_values_parse() {
        fn v<T: Into<Value>>(v: T) -> Value { v.into() }

        assert_parse_eq! {
            "[1,2,3]" => vec![1u8, 2u8, 3u8],
            "{a=b}" => map!["a" => "b"],
            "{\"a\"=b}" => map!["a" => "b"],
            "{\"a.b.c\"=b}" => map!["a.b.c" => "b"],
            "{a=1,b=3}" => map!["a" => 1u8, "b" => 3u8],
            "{a=1,b=hi}" => map!["a" => v(1u8), "b" => v("hi")],
            "[1,[2],3]" => vec![v(1u8), v(vec![2u8]), v(3u8)],
            "{a=[[-2]]}" => map!["a" => vec![vec![-2]]],
            "{a=[[-2]],b=\" hi\"}" => map!["a" => v(vec![vec![-2]]), "b" => v(" hi")],
            "[1,true,hi,\"a \"]" => vec![v(1u8), v(true), v("hi"), v("a ")],
            "[1,{a=b},hi]" => vec![v(1u8), v(map!["a" => "b"]), v("hi")],
            "[[ -1], {a=[ b ]},  hi ]" =>
                vec![v(vec![-1]), v(map!["a" => vec!["b"]]), v("hi")],
        }
    }
}
