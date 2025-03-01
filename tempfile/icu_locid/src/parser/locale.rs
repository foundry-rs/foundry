// This file is part of ICU4X. For terms of use, please see the file
// called LICENSE at the top level of the ICU4X source tree
// (online at: https://github.com/unicode-org/icu4x/blob/main/LICENSE ).

use tinystr::TinyAsciiStr;

use crate::extensions::{self, Extensions};
use crate::parser::errors::ParserError;
use crate::parser::{parse_language_identifier_from_iter, ParserMode, SubtagIterator};
use crate::{subtags, Locale};

use super::parse_locale_with_single_variant_single_keyword_unicode_extension_from_iter;

pub fn parse_locale(t: &[u8]) -> Result<Locale, ParserError> {
    let mut iter = SubtagIterator::new(t);

    let id = parse_language_identifier_from_iter(&mut iter, ParserMode::Locale)?;
    let extensions = if iter.peek().is_some() {
        Extensions::try_from_iter(&mut iter)?
    } else {
        Extensions::default()
    };
    Ok(Locale { id, extensions })
}

#[allow(clippy::type_complexity)]
pub const fn parse_locale_with_single_variant_single_keyword_unicode_keyword_extension(
    t: &[u8],
    mode: ParserMode,
) -> Result<
    (
        subtags::Language,
        Option<subtags::Script>,
        Option<subtags::Region>,
        Option<subtags::Variant>,
        Option<(extensions::unicode::Key, Option<TinyAsciiStr<8>>)>,
    ),
    ParserError,
> {
    let iter = SubtagIterator::new(t);
    parse_locale_with_single_variant_single_keyword_unicode_extension_from_iter(iter, mode)
}
