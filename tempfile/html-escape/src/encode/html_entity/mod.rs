mod unquoted_attribute;

use core::str::from_utf8_unchecked;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::io::{self, Write};

pub use unquoted_attribute::*;

macro_rules! escape_impl {
    (@inner [$dollar:tt] $name:ident; $($l:expr => $r:expr),+ $(,)*) => {
        macro_rules! $name {
            ($dollar e:expr) => {
                match $dollar e {
                    $($l => break $r,)+
                    _ => (),
                }
            };
            (vec $dollar e:expr, $dollar v:ident, $dollar b:ident, $dollar start:ident, $dollar end:ident) => {
                match $dollar e {
                    $($l => {
                        $dollar v.extend_from_slice(&$dollar b[$dollar start..$dollar end]);
                        $dollar start = $dollar end + 1;
                        $dollar v.extend_from_slice($r);
                    })+
                    _ => (),
                }

                $dollar end += 1;
            };
            (writer $dollar e:expr, $dollar w:ident, $dollar b:ident, $dollar start:ident, $dollar end:ident) => {
                match $dollar e {
                    $($l => {
                        $dollar w.write_all(&$dollar b[$dollar start..$dollar end])?;
                        $dollar start = $dollar end + 1;
                        $dollar w.write_all($r)?;
                    })+
                    _ => (),
                }

                $dollar end += 1;
            };
        }
    };
    ($name:ident; $($l:expr => $r:expr),+ $(,)*) => {
        escape_impl! {
            @inner [$]
            $name;
            $($l => $r.as_ref(),)*
        }
    };
}

escape_impl! {
    escape_text_minimal;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
}

escape_impl! {
    escape_text;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
    b'>' => b"&gt;",
}

escape_impl! {
    escape_double_quote;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
    b'>' => b"&gt;",
    b'"' => b"&quot;",
}

escape_impl! {
    escape_single_quote;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
    b'>' => b"&gt;",
    b'\'' => b"&#x27;",
}

escape_impl! {
    escape_quote;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
    b'>' => b"&gt;",
    b'"' => b"&quot;",
    b'\'' => b"&#x27;",
}

escape_impl! {
    escape_safe;
    b'&' => b"&amp;",
    b'<' => b"&lt;",
    b'>' => b"&gt;",
    b'"' => b"&quot;",
    b'\'' => b"&#x27;",
    b'/' => b"&#x2F;",
}

macro_rules! encode_impl {
    ($(#[$attr: meta])* $escape_macro:ident; $(#[$encode_attr: meta])* $encode_name: ident; $(#[$encode_to_string_attr: meta])* $encode_to_string_name: ident; $(#[$encode_to_vec_attr: meta])* $encode_to_vec_name: ident; $(#[$encode_to_writer_attr: meta])* $encode_to_writer_name: ident $(;)*) => {
        $(#[$encode_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_name<S: ?Sized + AsRef<str>>(text: &S) -> Cow<str> {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();
            let text_length = text_bytes.len();

            let mut p = 0;
            let mut e;

            let first = loop {
                if p == text_length {
                    return Cow::from(text);
                }

                e = text_bytes[p];

                $escape_macro!(e);

                p += 1;
            };

            let mut v = Vec::with_capacity(text_length + 5);

            v.extend_from_slice(&text_bytes[..p]);
            v.extend_from_slice(first);

            $encode_to_vec_name(unsafe { from_utf8_unchecked(&text_bytes[(p + 1)..]) }, &mut v);

            Cow::from(unsafe { String::from_utf8_unchecked(v) })
        }

        $(#[$encode_to_string_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_string_name<S: AsRef<str>>(text: S, output: &mut String) -> &str {
            unsafe { from_utf8_unchecked($encode_to_vec_name(text, output.as_mut_vec())) }
        }

        $(#[$encode_to_vec_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_vec_name<S: AsRef<str>>(text: S, output: &mut Vec<u8>) -> &[u8] {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();
            let text_length = text_bytes.len();

            output.reserve(text_length);

            let current_length = output.len();

            let mut start = 0;
            let mut end = 0;

            for e in text_bytes.iter().copied() {
                $escape_macro!(vec e, output, text_bytes, start, end);
            }

            output.extend_from_slice(&text_bytes[start..end]);

            &output[current_length..]
        }

        #[cfg(feature = "std")]
        $(#[$encode_to_writer_attr])*
        ///
        $(#[$attr])*
        #[inline]
        pub fn $encode_to_writer_name<S: AsRef<str>, W: Write>(text: S, output: &mut W) -> Result<(), io::Error> {
            let text = text.as_ref();
            let text_bytes = text.as_bytes();

            let mut start = 0;
            let mut end = 0;

            for e in text_bytes.iter().copied() {
                $escape_macro!(writer e, output, text_bytes, start, end);
            }

            output.write_all(&text_bytes[start..end])
        }
    };
}

encode_impl! {
    /// The following characters are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    escape_text_minimal;
    /// Encode text used as regular HTML text.
    encode_text_minimal;
    /// Write text used as regular HTML text to a mutable `String` reference and return the encoded string slice.
    encode_text_minimal_to_string;
    /// Write text used as regular HTML text to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_text_minimal_to_vec;
    /// Write text used as regular HTML text to a writer.
    encode_text_minimal_to_writer;
}

encode_impl! {
    /// The following characters are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    /// * `>` => `&gt;`
    escape_text;
    /// Encode text used as regular HTML text.
    encode_text;
    /// Write text used as regular HTML text to a mutable `String` reference and return the encoded string slice.
    encode_text_to_string;
    /// Write text used as regular HTML text to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_text_to_vec;
    /// Write text used as regular HTML text to a writer.
    encode_text_to_writer;
}

encode_impl! {
    /// The following characters are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    /// * `>` => `&gt;`
    /// * `"` => `&quot;`
    escape_double_quote;
    /// Encode text used in a double-quoted attribute.
    encode_double_quoted_attribute;
    /// Write text used in a double-quoted attribute to a mutable `String` reference and return the encoded string slice.
    encode_double_quoted_attribute_to_string;
    /// Write text used in a double-quoted attribute to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_double_quoted_attribute_to_vec;
    /// Write text used in a double-quoted attribute to a writer.
    encode_double_quoted_attribute_to_writer;
}

encode_impl! {
    /// The following characters are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    /// * `>` => `&gt;`
    /// * `'` => `&#x27;`
    escape_single_quote;
    /// Encode text used in a single-quoted attribute.
    encode_single_quoted_attribute;
    /// Write text used in a single-quoted attribute to a mutable `String` reference and return the encoded string slice.
    encode_single_quoted_attribute_to_string;
    /// Write text used in a single-quoted attribute to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_single_quoted_attribute_to_vec;
    /// Write text used in a single-quoted attribute to a writer.
    encode_single_quoted_attribute_to_writer;
}

encode_impl! {
    /// The following characters (HTML reserved characters)  are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    /// * `>` => `&gt;`
    /// * `"` => `&quot;`
    /// * `'` => `&#x27;`
    escape_quote;
    /// Encode text used in a quoted attribute.
    encode_quoted_attribute;
    /// Write text used in a quoted attribute to a mutable `String` reference and return the encoded string slice.
    encode_quoted_attribute_to_string;
    /// Write text used in a quoted attribute to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_quoted_attribute_to_vec;
    /// Write text used in a quoted attribute to a writer.
    encode_quoted_attribute_to_writer;
}

encode_impl! {
    /// The following characters are escaped:
    ///
    /// * `&` => `&amp;`
    /// * `<` => `&lt;`
    /// * `>` => `&gt;`
    /// * `"` => `&quot;`
    /// * `'` => `&#x27;`
    /// * `/` => `&#x2F;`
    escape_safe;
    /// Encode text to prevent special characters functioning.
    encode_safe;
    /// Encode text to prevent special characters functioning and write it to a mutable `String` reference and return the encoded string slice.
    encode_safe_to_string;
    /// Encode text to prevent special characters functioning and write it to a mutable `Vec<u8>` reference and return the encoded data slice.
    encode_safe_to_vec;
    /// Encode text to prevent special characters functioning and write it to a writer.
    encode_safe_to_writer;
}
