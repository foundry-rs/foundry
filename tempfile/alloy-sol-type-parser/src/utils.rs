#![allow(missing_docs)]

use crate::{
    ident::identifier_parser, input::check_recursion, new_input, Error, Input, ParameterSpecifier,
    Result, RootType, StateMutability,
};
use alloc::{string::String, vec::Vec};
use core::{slice, str};
use winnow::{
    ascii::space0,
    combinator::{alt, cut_err, opt, preceded, separated, terminated, trace},
    error::{ContextError, ParserError, StrContext, StrContextValue},
    stream::{Accumulate, AsChar, Stream},
    ModalParser, ModalResult, Parser,
};

pub use crate::ident::identifier;

#[inline]
pub fn spanned<'a, O, E: ParserError<Input<'a>>>(
    mut f: impl ModalParser<Input<'a>, O, E>,
) -> impl ModalParser<Input<'a>, (&'a str, O), E> {
    trace("spanned", move |input: &mut Input<'a>| {
        let start = input.as_ptr();

        let mut len = input.len();
        let r = f.parse_next(input)?;
        len -= input.len();

        // SAFETY: str invariant
        unsafe {
            let span = slice::from_raw_parts(start, len);
            debug_assert!(str::from_utf8(span).is_ok());
            Ok((str::from_utf8_unchecked(span), r))
        }
    })
}

#[inline]
pub fn char_parser<'a>(c: char) -> impl ModalParser<Input<'a>, char, ContextError> {
    #[cfg(feature = "debug")]
    let name = format!("char={c:?}");
    #[cfg(not(feature = "debug"))]
    let name = "char";
    trace(name, c.context(StrContext::Expected(StrContextValue::CharLiteral(c))))
}

#[inline]
pub fn str_parser<'a>(s: &'static str) -> impl ModalParser<Input<'a>, &'a str, ContextError> {
    #[cfg(feature = "debug")]
    let name = format!("str={s:?}");
    #[cfg(not(feature = "debug"))]
    let name = "str";
    trace(name, s.context(StrContext::Expected(StrContextValue::StringLiteral(s))))
}

pub fn tuple_parser<'a, O1, O2: Accumulate<O1>>(
    f: impl ModalParser<Input<'a>, O1, ContextError>,
) -> impl ModalParser<Input<'a>, O2, ContextError> {
    list_parser('(', ',', ')', f)
}

pub fn array_parser<'a, O1, O2: Accumulate<O1>>(
    f: impl ModalParser<Input<'a>, O1, ContextError>,
) -> impl ModalParser<Input<'a>, O2, ContextError> {
    list_parser('[', ',', ']', f)
}

#[inline]
fn list_parser<'i, O1, O2>(
    open: char,
    delim: char,
    close: char,
    f: impl ModalParser<Input<'i>, O1, ContextError>,
) -> impl ModalParser<Input<'i>, O2, ContextError>
where
    O2: Accumulate<O1>,
{
    #[cfg(feature = "debug")]
    let name = format!("list({open:?}, {delim:?}, {close:?})");
    #[cfg(not(feature = "debug"))]
    let name = "list";

    // These have to be outside of the closure for some reason.
    let f = check_recursion(f);
    let elems_1 = separated(1.., f, (char_parser(delim), space0));
    let mut elems_and_end = terminated(elems_1, (opt(delim), space0, cut_err(char_parser(close))));
    trace(name, move |input: &mut Input<'i>| {
        let _ = char_parser(open).parse_next(input)?;
        let _ = space0(input)?;
        if input.starts_with(close) {
            input.next_slice(close.len());
            return Ok(O2::initial(Some(0)));
        }
        elems_and_end.parse_next(input)
    })
}

pub fn opt_ws_ident<'a>(input: &mut Input<'a>) -> ModalResult<Option<&'a str>> {
    preceded(space0, opt(identifier_parser)).parse_next(input)
}

// Not public API.
#[doc(hidden)]
#[inline]
pub fn parse_item_keyword<'a>(s: &mut &'a str) -> Result<&'a str> {
    trace("item", terminated(identifier, space0)).parse_next(s).map_err(Error::parser)
}

#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedSignature<Param> {
    pub name: String,
    pub inputs: Vec<Param>,
    pub outputs: Vec<Param>,
    pub anonymous: bool,
    pub state_mutability: Option<StateMutability>,
}

#[doc(hidden)]
pub fn parse_signature<'a, const OUT: bool, F: Fn(ParameterSpecifier<'a>) -> T, T>(
    s: &'a str,
    f: F,
) -> Result<ParsedSignature<T>> {
    let params = || tuple_parser(ParameterSpecifier::parser.map(&f));
    trace(
        "signature",
        (
            // name
            RootType::parser,
            // inputs
            preceded(space0, params()),
            // visibility
            opt(preceded(space0, alt(("internal", "external", "private", "public")))),
            // state mutability
            opt(preceded(space0, alt(("pure", "view", "payable")))),
            // outputs
            move |i: &mut _| {
                if OUT {
                    preceded((space0, opt(":"), opt("returns"), space0), opt(params()))
                        .parse_next(i)
                        .map(Option::unwrap_or_default)
                } else {
                    Ok(vec![])
                }
            },
            // anonymous
            preceded(space0, opt("anonymous").map(|x| x.is_some())),
        ),
    )
    .map(|(name, inputs, _visibility, mutability, outputs, anonymous)| ParsedSignature {
        name: name.span().into(),
        inputs,
        outputs,
        anonymous,
        state_mutability: mutability.map(|s| s.parse().unwrap()),
    })
    .parse(new_input(s))
    .map_err(Error::parser)
}
