use core::str::from_utf8_unchecked;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::io::{self, Write};

use crate::functions::*;

/// Encode text used in an unquoted attribute. Except for alphanumeric characters, escape all characters which are less than 128.
///
/// The following characters are escaped to named entities:
///
/// * `&` => `&amp;`
/// * `<` => `&lt;`
/// * `>` => `&gt;`
/// * `"` => `&quot;`
///
/// Other non-alphanumeric characters are escaped to `&#xHH;`.
pub fn encode_unquoted_attribute<S: ?Sized + AsRef<str>>(text: &S) -> Cow<str> {
    let text = text.as_ref();
    let text_bytes = text.as_bytes();

    let text_length = text_bytes.len();

    let mut p = 0;
    let mut e;

    loop {
        if p == text_length {
            return Cow::from(text);
        }

        e = text_bytes[p];

        if utf8_width::is_width_1(e) && !e.is_ascii_alphanumeric() {
            break;
        }

        p += 1;
    }

    let mut v = Vec::with_capacity(text_length);

    v.extend_from_slice(&text_bytes[..p]);

    write_html_entity_to_vec(e, &mut v);

    encode_unquoted_attribute_to_vec(
        unsafe { from_utf8_unchecked(&text_bytes[(p + 1)..]) },
        &mut v,
    );

    Cow::from(unsafe { String::from_utf8_unchecked(v) })
}

/// Write text used in an unquoted attribute to a mutable `String` reference and return the encoded string slice. Except for alphanumeric characters, escape all characters which are less than 128.
///
/// The following characters are escaped to named entities:
///
/// * `&` => `&amp;`
/// * `<` => `&lt;`
/// * `>` => `&gt;`
/// * `"` => `&quot;`
///
/// Other non-alphanumeric characters are escaped to `&#xHH;`.
#[inline]
pub fn encode_unquoted_attribute_to_string<S: AsRef<str>>(text: S, output: &mut String) -> &str {
    unsafe { from_utf8_unchecked(encode_unquoted_attribute_to_vec(text, output.as_mut_vec())) }
}

/// Write text used in an unquoted attribute to a mutable `Vec<u8>` reference and return the encoded data slice. Except for alphanumeric characters, escape all characters which are less than 128.
///
/// The following characters are escaped to named entities:
///
/// * `&` => `&amp;`
/// * `<` => `&lt;`
/// * `>` => `&gt;`
/// * `"` => `&quot;`
///
/// Other non-alphanumeric characters are escaped to `&#xHH;`.
pub fn encode_unquoted_attribute_to_vec<S: AsRef<str>>(text: S, output: &mut Vec<u8>) -> &[u8] {
    let text = text.as_ref();
    let text_bytes = text.as_bytes();
    let text_length = text_bytes.len();

    output.reserve(text_length);

    let current_length = output.len();

    let mut p = 0;
    let mut e;

    let mut start = 0;

    while p < text_length {
        e = text_bytes[p];

        if utf8_width::is_width_1(e) && !e.is_ascii_alphanumeric() {
            output.extend_from_slice(&text_bytes[start..p]);
            start = p + 1;
            write_html_entity_to_vec(e, output);
        }

        p += 1;
    }

    output.extend_from_slice(&text_bytes[start..p]);

    &output[current_length..]
}

#[cfg(feature = "std")]
/// Write text used in an unquoted attribute to a writer. Except for alphanumeric characters, escape all characters which are less than 128.
///
/// The following characters are escaped to named entities:
///
/// * `&` => `&amp;`
/// * `<` => `&lt;`
/// * `>` => `&gt;`
/// * `"` => `&quot;`
///
/// Other non-alphanumeric characters are escaped to `&#xHH;`.
pub fn encode_unquoted_attribute_to_writer<S: AsRef<str>, W: Write>(
    text: S,
    output: &mut W,
) -> Result<(), io::Error> {
    let text = text.as_ref();
    let text_bytes = text.as_bytes();
    let text_length = text_bytes.len();

    let mut p = 0;
    let mut e;

    let mut start = 0;

    while p < text_length {
        e = text_bytes[p];

        if utf8_width::is_width_1(e) && !e.is_ascii_alphanumeric() {
            output.write_all(&text_bytes[start..p])?;
            start = p + 1;
            write_html_entity_to_writer(e, output)?;
        }

        p += 1;
    }

    output.write_all(&text_bytes[start..p])
}
