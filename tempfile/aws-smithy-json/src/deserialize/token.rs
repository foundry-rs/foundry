/*
 * Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use crate::deserialize::error::DeserializeError as Error;
use crate::deserialize::must_not_be_finite;
use crate::escape::unescape_string;
pub use crate::escape::EscapeError;
use aws_smithy_types::date_time::Format;
use aws_smithy_types::primitive::Parse;
use aws_smithy_types::{base64, Blob, DateTime, Document, Number};
use std::borrow::Cow;
use std::collections::HashMap;
use std::iter::Peekable;

/// New-type around `&str` that indicates the string is an escaped JSON string.
/// Provides functions for retrieving the string in either form.
#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub struct EscapedStr<'a>(&'a str);

impl<'a> EscapedStr<'a> {
    pub fn new(value: &'a str) -> EscapedStr<'a> {
        EscapedStr(value)
    }

    /// Returns the escaped string value
    pub fn as_escaped_str(&self) -> &'a str {
        self.0
    }

    /// Unescapes the string and returns it.
    /// If the string doesn't need unescaping, it will be returned directly.
    pub fn to_unescaped(self) -> Result<Cow<'a, str>, EscapeError> {
        unescape_string(self.0)
    }
}

/// Represents the location of a token
#[derive(Debug, Eq, PartialEq, Copy, Clone)]
pub struct Offset(pub usize);

impl Offset {
    /// Creates a custom error from the offset
    pub fn error(&self, msg: Cow<'static, str>) -> Error {
        Error::custom(msg).with_offset(self.0)
    }
}

/// Enum representing the different JSON tokens that can be returned by
/// [`crate::deserialize::json_token_iter`].
#[derive(Debug, PartialEq)]
pub enum Token<'a> {
    StartArray {
        offset: Offset,
    },
    EndArray {
        offset: Offset,
    },
    ObjectKey {
        offset: Offset,
        key: EscapedStr<'a>,
    },
    StartObject {
        offset: Offset,
    },
    EndObject {
        offset: Offset,
    },
    ValueBool {
        offset: Offset,
        value: bool,
    },
    ValueNull {
        offset: Offset,
    },
    ValueNumber {
        offset: Offset,
        value: Number,
    },
    ValueString {
        offset: Offset,
        value: EscapedStr<'a>,
    },
}

impl<'a> Token<'a> {
    pub fn offset(&self) -> Offset {
        use Token::*;
        *match self {
            StartArray { offset } => offset,
            EndArray { offset } => offset,
            ObjectKey { offset, .. } => offset,
            StartObject { offset } => offset,
            EndObject { offset } => offset,
            ValueBool { offset, .. } => offset,
            ValueNull { offset } => offset,
            ValueNumber { offset, .. } => offset,
            ValueString { offset, .. } => offset,
        }
    }

    /// Builds an error from the token's offset
    pub fn error(&self, msg: Cow<'static, str>) -> Error {
        self.offset().error(msg)
    }
}

macro_rules! expect_fn {
    ($name:ident, $token:ident, $doc:tt) => {
        #[doc=$doc]
        pub fn $name(token_result: Option<Result<Token<'_>, Error>>) -> Result<(), Error> {
            match token_result.transpose()? {
                Some(Token::$token { .. }) => Ok(()),
                Some(token) => {
                    Err(token.error(Cow::Borrowed(concat!("expected ", stringify!($token)))))
                }
                None => Err(Error::custom(concat!("expected ", stringify!($token)))),
            }
        }
    };
}

expect_fn!(
    expect_start_object,
    StartObject,
    "Expects a [Token::StartObject] token and returns an error if it's not present."
);
expect_fn!(
    expect_start_array,
    StartArray,
    "Expects a [Token::StartArray] token and returns an error if it's not present."
);

macro_rules! expect_value_or_null_fn {
    ($name:ident, $token:ident, $typ:ident, $doc:tt) => {
        #[doc=$doc]
        pub fn $name(token: Option<Result<Token<'_>, Error>>) -> Result<Option<$typ>, Error> {
            match token.transpose()? {
                Some(Token::ValueNull { .. }) => Ok(None),
                Some(Token::$token { value, .. }) => Ok(Some(value)),
                _ => Err(Error::custom(concat!(
                    "expected ",
                    stringify!($token),
                    " or ValueNull"
                ))),
            }
        }
    };
}

expect_value_or_null_fn!(expect_bool_or_null, ValueBool, bool, "Expects a [Token::ValueBool] or [Token::ValueNull], and returns the bool value if it's not null.");
expect_value_or_null_fn!(expect_string_or_null, ValueString, EscapedStr, "Expects a [Token::ValueString] or [Token::ValueNull], and returns the [EscapedStr] value if it's not null.");

/// Expects a [Token::ValueString], [Token::ValueNumber] or [Token::ValueNull].
///
/// If the value is a string, it MUST be `Infinity`, `-Infinity` or `Nan`.
/// If the value is a number, it is returned directly
pub fn expect_number_or_null(
    token: Option<Result<Token<'_>, Error>>,
) -> Result<Option<Number>, Error> {
    match token.transpose()? {
        Some(Token::ValueNull { .. }) => Ok(None),
        Some(Token::ValueNumber { value, .. }) => Ok(Some(value)),
        Some(Token::ValueString { value, offset }) => match value.to_unescaped() {
            Err(err) => Err(Error::custom_source( "expected a valid string, escape was invalid", err).with_offset(offset.0)),
            Ok(v) => f64::parse_smithy_primitive(v.as_ref())
                // disregard the exact error
                .map_err(|_|())
                // only infinite / NaN can be used as strings
                .and_then(must_not_be_finite)
                .map(|float| Some(aws_smithy_types::Number::Float(float)))
                // convert to a helpful error
                .map_err(|_| {
                    Error::custom(
                        format!(
                        "only `Infinity`, `-Infinity`, `NaN` can represent a float as a string but found `{}`",
                        v
                    )).with_offset(offset.0)
                }),
        },
        _ => Err(Error::custom(
            "expected ValueString, ValueNumber, or ValueNull",
        )),
    }
}

/// Expects a [Token::ValueString] or [Token::ValueNull]. If the value is a string, it interprets it as a base64 encoded [Blob] value.
pub fn expect_blob_or_null(token: Option<Result<Token<'_>, Error>>) -> Result<Option<Blob>, Error> {
    Ok(match expect_string_or_null(token)? {
        Some(value) => Some(Blob::new(
            base64::decode(value.as_escaped_str())
                .map_err(|err| Error::custom_source("failed to decode base64", err))?,
        )),
        None => None,
    })
}

/// Expects a [Token::ValueNull], [Token::ValueString], or [Token::ValueNumber] depending
/// on the passed in `timestamp_format`. If there is a non-null value, it interprets it as an
/// [`DateTime` ] in the requested format.
pub fn expect_timestamp_or_null(
    token: Option<Result<Token<'_>, Error>>,
    timestamp_format: Format,
) -> Result<Option<DateTime>, Error> {
    Ok(match timestamp_format {
        Format::EpochSeconds => expect_number_or_null(token)?
            .map(|v| v.to_f64_lossy())
            .map(|v| {
                if v.is_nan() {
                    Err(Error::custom("NaN is not a valid epoch"))
                } else if v.is_infinite() {
                    Err(Error::custom("infinity is not a valid epoch"))
                } else {
                    Ok(DateTime::from_secs_f64(v))
                }
            })
            .transpose()?,
        Format::DateTime | Format::HttpDate | Format::DateTimeWithOffset => {
            expect_string_or_null(token)?
                .map(|v| DateTime::from_str(v.as_escaped_str(), timestamp_format))
                .transpose()
                .map_err(|err| Error::custom_source("failed to parse timestamp", err))?
        }
    })
}

/// Expects and parses a complete document value.
pub fn expect_document<'a, I>(tokens: &mut Peekable<I>) -> Result<Document, Error>
where
    I: Iterator<Item = Result<Token<'a>, Error>>,
{
    expect_document_inner(tokens, 0)
}

const MAX_DOCUMENT_RECURSION: usize = 256;

fn expect_document_inner<'a, I>(tokens: &mut Peekable<I>, depth: usize) -> Result<Document, Error>
where
    I: Iterator<Item = Result<Token<'a>, Error>>,
{
    if depth >= MAX_DOCUMENT_RECURSION {
        return Err(Error::custom(
            "exceeded max recursion depth while parsing document",
        ));
    }
    match tokens.next().transpose()? {
        Some(Token::ValueNull { .. }) => Ok(Document::Null),
        Some(Token::ValueBool { value, .. }) => Ok(Document::Bool(value)),
        Some(Token::ValueNumber { value, .. }) => Ok(Document::Number(value)),
        Some(Token::ValueString { value, .. }) => {
            Ok(Document::String(value.to_unescaped()?.into_owned()))
        }
        Some(Token::StartObject { .. }) => {
            let mut object = HashMap::new();
            loop {
                match tokens.next().transpose()? {
                    Some(Token::EndObject { .. }) => break,
                    Some(Token::ObjectKey { key, .. }) => {
                        let key = key.to_unescaped()?.into_owned();
                        let value = expect_document_inner(tokens, depth + 1)?;
                        object.insert(key, value);
                    }
                    _ => return Err(Error::custom("expected object key or end object")),
                }
            }
            Ok(Document::Object(object))
        }
        Some(Token::StartArray { .. }) => {
            let mut array = Vec::new();
            loop {
                match tokens.peek() {
                    Some(Ok(Token::EndArray { .. })) => {
                        tokens.next().transpose().unwrap();
                        break;
                    }
                    _ => array.push(expect_document_inner(tokens, depth + 1)?),
                }
            }
            Ok(Document::Array(array))
        }
        Some(Token::EndObject { .. }) | Some(Token::ObjectKey { .. }) => {
            unreachable!("end object and object key are handled in start object")
        }
        Some(Token::EndArray { .. }) => unreachable!("end array is handled in start array"),
        None => Err(Error::custom("expected value")),
    }
}

/// Skips an entire value in the token stream. Errors if it isn't a value.
pub fn skip_value<'a>(
    tokens: &mut impl Iterator<Item = Result<Token<'a>, Error>>,
) -> Result<(), Error> {
    skip_inner(0, tokens)
}

/// Assumes a start object/array token has already been consumed and skips tokens until
/// until its corresponding end object/array token is found.
pub fn skip_to_end<'a>(
    tokens: &mut impl Iterator<Item = Result<Token<'a>, Error>>,
) -> Result<(), Error> {
    skip_inner(1, tokens)
}

fn skip_inner<'a>(
    depth: isize,
    tokens: &mut impl Iterator<Item = Result<Token<'a>, Error>>,
) -> Result<(), Error> {
    loop {
        match tokens.next().transpose()? {
            Some(Token::StartObject { .. }) | Some(Token::StartArray { .. }) => {
                skip_inner(depth + 1, tokens)?;
                if depth == 0 {
                    break;
                }
            }
            Some(Token::EndObject { .. }) | Some(Token::EndArray { .. }) => {
                debug_assert!(depth > 0);
                break;
            }
            Some(Token::ValueNull { .. })
            | Some(Token::ValueBool { .. })
            | Some(Token::ValueNumber { .. })
            | Some(Token::ValueString { .. }) => {
                if depth == 0 {
                    break;
                }
            }
            Some(Token::ObjectKey { .. }) => {}
            _ => return Err(Error::custom("expected value")),
        }
    }
    Ok(())
}

#[cfg(test)]
pub mod test {
    use super::*;
    use crate::deserialize::error::DeserializeErrorKind as ErrorKind;
    use crate::deserialize::error::DeserializeErrorKind::UnexpectedToken;
    use crate::deserialize::json_token_iter;

    pub fn start_array<'a>(offset: usize) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::StartArray {
            offset: Offset(offset),
        }))
    }

    pub fn end_array<'a>(offset: usize) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::EndArray {
            offset: Offset(offset),
        }))
    }

    pub fn start_object<'a>(offset: usize) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::StartObject {
            offset: Offset(offset),
        }))
    }

    pub fn end_object<'a>(offset: usize) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::EndObject {
            offset: Offset(offset),
        }))
    }

    pub fn object_key(offset: usize, key: &str) -> Option<Result<Token<'_>, Error>> {
        Some(Ok(Token::ObjectKey {
            offset: Offset(offset),
            key: EscapedStr::new(key),
        }))
    }

    pub fn value_bool<'a>(offset: usize, boolean: bool) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::ValueBool {
            offset: Offset(offset),
            value: boolean,
        }))
    }

    pub fn value_number<'a>(offset: usize, number: Number) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::ValueNumber {
            offset: Offset(offset),
            value: number,
        }))
    }

    pub fn value_null<'a>(offset: usize) -> Option<Result<Token<'a>, Error>> {
        Some(Ok(Token::ValueNull {
            offset: Offset(offset),
        }))
    }

    pub fn value_string(offset: usize, string: &str) -> Option<Result<Token<'_>, Error>> {
        Some(Ok(Token::ValueString {
            offset: Offset(offset),
            value: EscapedStr::new(string),
        }))
    }

    #[track_caller]
    fn expect_err_custom<T>(message: &str, offset: Option<usize>, result: Result<T, Error>) {
        let err = result.err().expect("expected error");
        let (actual_message, actual_offset) = match &err.kind {
            ErrorKind::Custom { message, .. } => (message.as_ref(), err.offset),
            _ => panic!("expected ErrorKind::Custom, got {:?}", err),
        };
        assert_eq!((message, offset), (actual_message, actual_offset));
    }

    #[test]
    fn skip_simple_value() {
        let mut tokens = json_token_iter(b"null true");
        skip_value(&mut tokens).unwrap();
        assert!(matches!(
            tokens.next(),
            Some(Ok(Token::ValueBool { value: true, .. }))
        ))
    }

    #[test]
    fn skip_array() {
        let mut tokens = json_token_iter(b"[1, 2, 3, 4] true");
        skip_value(&mut tokens).unwrap();
        assert!(matches!(
            tokens.next(),
            Some(Ok(Token::ValueBool { value: true, .. }))
        ))
    }

    #[test]
    fn skip_object() {
        let mut tokens = json_token_iter(b"{\"one\": 5, \"two\": 3} true");
        skip_value(&mut tokens).unwrap();
        assert!(matches!(
            tokens.next(),
            Some(Ok(Token::ValueBool { value: true, .. }))
        ))
    }

    #[test]
    fn test_skip_to_end() {
        let tokens = json_token_iter(b"{\"one\": { \"two\": [] }, \"three\":2 }");
        let mut tokens = tokens.skip(2);
        assert!(matches!(tokens.next(), Some(Ok(Token::StartObject { .. }))));
        skip_to_end(&mut tokens).unwrap();
        match tokens.next() {
            Some(Ok(Token::ObjectKey { key, .. })) => {
                assert_eq!("three", key.as_escaped_str());
            }
            _ => panic!("expected object key three"),
        }
    }

    #[test]
    fn test_non_finite_floats() {
        let mut tokens = json_token_iter(b"inf");
        tokens
            .next()
            .expect("there is a token")
            .expect_err("but it is invalid, ensure that Rust float boundary cases don't parse");
    }

    #[test]
    fn mismatched_braces() {
        // The skip_value function doesn't need to explicitly handle these cases since
        // token iterator's parser handles them. This test confirms that assumption.
        assert!(matches!(
            skip_value(&mut json_token_iter(br#"[{"foo": 5]}"#)),
            Err(Error {
                kind: UnexpectedToken(']', "'}', ','"),
                offset: Some(10)
            })
        ));
        assert!(matches!(
            skip_value(&mut json_token_iter(br#"{"foo": 5]}"#)),
            Err(Error {
                kind: UnexpectedToken(']', "'}', ','"),
                offset: Some(9)
            })
        ));
        assert!(matches!(
            skip_value(&mut json_token_iter(br#"[5,6}"#)),
            Err(Error {
                kind: UnexpectedToken('}', "']', ','"),
                offset: Some(4)
            })
        ));
    }

    #[test]
    fn skip_nested() {
        let mut tokens = json_token_iter(
            br#"
            {"struct": {"foo": 5, "bar": 11, "arr": [1, 2, 3, {}, 5, []]},
             "arr": [[], [[]], [{"arr":[]}]],
             "simple": "foo"}
            true
        "#,
        );
        skip_value(&mut tokens).unwrap();
        assert!(matches!(
            tokens.next(),
            Some(Ok(Token::ValueBool { value: true, .. }))
        ))
    }

    #[test]
    fn test_expect_start_object() {
        expect_err_custom(
            "expected StartObject",
            Some(2),
            expect_start_object(value_bool(2, true)),
        );
        assert!(expect_start_object(start_object(0)).is_ok());
    }

    #[test]
    fn test_expect_start_array() {
        expect_err_custom(
            "expected StartArray",
            Some(2),
            expect_start_array(value_bool(2, true)),
        );
        assert!(expect_start_array(start_array(0)).is_ok());
    }

    #[test]
    fn test_expect_string_or_null() {
        assert_eq!(None, expect_string_or_null(value_null(0)).unwrap());
        assert_eq!(
            Some(EscapedStr("test\\n")),
            expect_string_or_null(value_string(0, "test\\n")).unwrap()
        );
        expect_err_custom(
            "expected ValueString or ValueNull",
            None,
            expect_string_or_null(value_bool(0, true)),
        );
    }

    #[test]
    fn test_expect_number_or_null() {
        assert_eq!(None, expect_number_or_null(value_null(0)).unwrap());
        assert_eq!(
            Some(Number::PosInt(5)),
            expect_number_or_null(value_number(0, Number::PosInt(5))).unwrap()
        );
        expect_err_custom(
            "expected ValueString, ValueNumber, or ValueNull",
            None,
            expect_number_or_null(value_bool(0, true)),
        );
        assert_eq!(
            Some(Number::Float(f64::INFINITY)),
            expect_number_or_null(value_string(0, "Infinity")).unwrap()
        );
        expect_err_custom(
            "only `Infinity`, `-Infinity`, `NaN` can represent a float as a string but found `123`",
            Some(0),
            expect_number_or_null(value_string(0, "123")),
        );
        match expect_number_or_null(value_string(0, "NaN")) {
            Ok(Some(Number::Float(v))) if v.is_nan() => {
                // ok
            }
            not_ok => {
                panic!("expected nan, found: {:?}", not_ok)
            }
        }
    }

    #[test]
    fn test_expect_blob_or_null() {
        assert_eq!(None, expect_blob_or_null(value_null(0)).unwrap());
        assert_eq!(
            Some(Blob::new(b"hello!".to_vec())),
            expect_blob_or_null(value_string(0, "aGVsbG8h")).unwrap()
        );
        expect_err_custom(
            "expected ValueString or ValueNull",
            None,
            expect_blob_or_null(value_bool(0, true)),
        );
    }

    #[test]
    fn test_expect_timestamp_or_null() {
        assert_eq!(
            None,
            expect_timestamp_or_null(value_null(0), Format::HttpDate).unwrap()
        );
        for (invalid, display_name) in &[
            ("NaN", "NaN"),
            ("Infinity", "infinity"),
            ("-Infinity", "infinity"),
        ] {
            expect_err_custom(
                format!("{display_name} is not a valid epoch").as_str(),
                None,
                expect_timestamp_or_null(value_string(0, invalid), Format::EpochSeconds),
            );
        }
        assert_eq!(
            Some(DateTime::from_secs_f64(2048.0)),
            expect_timestamp_or_null(value_number(0, Number::Float(2048.0)), Format::EpochSeconds)
                .unwrap()
        );
        assert_eq!(
            Some(DateTime::from_secs_f64(1445412480.0)),
            expect_timestamp_or_null(
                value_string(0, "Wed, 21 Oct 2015 07:28:00 GMT"),
                Format::HttpDate
            )
            .unwrap()
        );
        assert_eq!(
            Some(DateTime::from_secs_f64(1445412480.0)),
            expect_timestamp_or_null(value_string(0, "2015-10-21T07:28:00Z"), Format::DateTime)
                .unwrap()
        );
        expect_err_custom(
                "only `Infinity`, `-Infinity`, `NaN` can represent a float as a string but found `wrong`",
                Some(0),
            expect_timestamp_or_null(value_string(0, "wrong"), Format::EpochSeconds)
        );
        expect_err_custom(
            "expected ValueString or ValueNull",
            None,
            expect_timestamp_or_null(value_number(0, Number::Float(0.0)), Format::DateTime),
        );
    }

    #[test]
    fn test_expect_document() {
        let test = |value| expect_document(&mut json_token_iter(value).peekable()).unwrap();
        assert_eq!(Document::Null, test(b"null"));
        assert_eq!(Document::Bool(true), test(b"true"));
        assert_eq!(Document::Number(Number::Float(3.2)), test(b"3.2"));
        assert_eq!(Document::String("Foo\nBar".into()), test(b"\"Foo\\nBar\""));
        assert_eq!(Document::Array(Vec::new()), test(b"[]"));
        assert_eq!(Document::Object(HashMap::new()), test(b"{}"));
        assert_eq!(
            Document::Array(vec![
                Document::Number(Number::PosInt(1)),
                Document::Bool(false),
                Document::String("s".into()),
                Document::Array(Vec::new()),
                Document::Object(HashMap::new()),
            ]),
            test(b"[1,false,\"s\",[],{}]")
        );
        assert_eq!(
            Document::Object(
                vec![
                    ("num".to_string(), Document::Number(Number::PosInt(1))),
                    ("bool".to_string(), Document::Bool(true)),
                    ("string".to_string(), Document::String("s".into())),
                    (
                        "array".to_string(),
                        Document::Array(vec![
                            Document::Object(
                                vec![("foo".to_string(), Document::Bool(false))]
                                    .into_iter()
                                    .collect(),
                            ),
                            Document::Object(
                                vec![("bar".to_string(), Document::Bool(true))]
                                    .into_iter()
                                    .collect(),
                            ),
                        ])
                    ),
                    (
                        "nested".to_string(),
                        Document::Object(
                            vec![("test".to_string(), Document::Null),]
                                .into_iter()
                                .collect()
                        )
                    ),
                ]
                .into_iter()
                .collect()
            ),
            test(
                br#"
                { "num": 1,
                  "bool": true,
                  "string": "s",
                  "array":
                      [{ "foo": false },
                       { "bar": true }],
                  "nested": { "test": null } }
                "#
            )
        );
    }

    #[test]
    fn test_document_recursion_limit() {
        let mut value = String::new();
        value.extend(std::iter::repeat('[').take(300));
        value.extend(std::iter::repeat(']').take(300));
        expect_err_custom(
            "exceeded max recursion depth while parsing document",
            None,
            expect_document(&mut json_token_iter(value.as_bytes()).peekable()),
        );

        value = String::new();
        value.extend(std::iter::repeat("{\"t\":").take(300));
        value.push('1');
        value.extend(std::iter::repeat('}').take(300));
        expect_err_custom(
            "exceeded max recursion depth while parsing document",
            None,
            expect_document(&mut json_token_iter(value.as_bytes()).peekable()),
        );
    }
}
