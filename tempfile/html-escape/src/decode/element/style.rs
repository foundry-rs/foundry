use core::str::from_utf8_unchecked;

use alloc::borrow::Cow;
use alloc::string::String;
use alloc::vec::Vec;

#[cfg(feature = "std")]
use std::io::{self, Write};

macro_rules! parse_style {
    ($e:expr, $step:ident, $b:block, $bq:block, $bc:block $(, $($addi:expr),+)?) => {
        match $step {
            0 => {
                match $e {
                    b'<' => $step = 1,
                    b'\\' => $step = 100,
                    _ => (),
                }
            }
            1 => {
                match $e {
                    b'\\' => $step = 2,
                    _ => (),
                }
            }
            2 => {
                match $e {
                    b'/' => $step = 3,
                    b'!' => $step = 10,
                    $($(| $addi)+ => {
                        $step = 0;
                        $bq
                    },)?
                    _ => $step = 0,
                }
            }
            3 => {
                match $e {
                    b's' | b'S' => $step = 4,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            4 => {
                match $e {
                    b't' | b'T' => $step = 5,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            5 => {
                match $e {
                    b'y' | b'Y' => $step = 6,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            6 => {
                match $e {
                    b'l' | b'L' => $step = 7,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            7 => {
                match $e {
                    b'e' | b'E' => $step = 8,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            8 => {
                match $e {
                    b'>' | 9..=13 | 28..=32 => {
                        $step = 0;
                        $b
                    },
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            10 => {
                match $e {
                    b'-' => $step = 11,
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            11 => {
                match $e {
                    b'-' => {
                        $step = 0;
                        $bc
                    },
                    b'\\' => $step = 100,
                    _ => $step = 0,
                }
            }
            100 => {
                match $e {
                    b'<' => $step = 1,
                     $($(| $addi)+ => {
                        $step = 0;
                        $bq
                    },)?
                    _ => $step = 0,
                }
            }
            _ => unreachable!(),
        }
    };
}

macro_rules! parse_style_single_quoted_text {
    ($e:expr, $step:ident, $b:block, $bq:block, $bc:block) => {
        parse_style!($e, $step, $b, $bq, $bc, b'\'');
    };
}

macro_rules! parse_style_double_quoted_text {
    ($e:expr, $step:ident, $b:block, $bq:block, $bc:block) => {
        parse_style!($e, $step, $b, $bq, $bc, b'"');
    };
}

macro_rules! parse_style_quoted_text {
    ($e:expr, $step:ident, $b:block, $bq:block, $bc:block) => {
        parse_style!($e, $step, $b, $bq, $bc, b'\'', b'"');
    };
}

decode_impl! {
    6;
    /// The following substring is unescaped:
    ///
    /// * `<\/style>` => `</style>`
    parse_style;
    /// Decode text from the `<style>` element.
    decode_style;
    /// Write text from the `<style>` element to a mutable `String` reference and return the encoded string slice.
    decode_style_to_string;
    /// Write text from the `<style>` element to a mutable `Vec<u8>` reference and return the encoded data slice.
    decode_style_to_vec;
    /// Write text from the `<style>` element to a writer.
    decode_style_to_writer;
}

decode_impl! {
    6;
    /// The following substring and character is unescaped:
    ///
    /// * `<\/style>` => `</style>`
    /// * `\'` => `'`
    parse_style_single_quoted_text;
    /// Decode text from a single quoted text in the `<style>` element.
    decode_style_single_quoted_text;
    /// Write text from a single quoted text in the `<style>` element to a mutable `String` reference and return the encoded string slice.
    decode_style_single_quoted_text_to_string;
    /// Write text from a single quoted text in the `<style>` element to a mutable `Vec<u8>` reference and return the encoded data slice.
    decode_style_single_quoted_text_to_vec;
    /// Write text from a single quoted text in the `<style>` element to a writer.
    decode_style_single_quoted_text_to_writer;
}

decode_impl! {
    6;
    /// The following substring and character are unescaped:
    ///
    /// * `<\/style>` => `</style>`
    /// * `\"` => `"`
    parse_style_double_quoted_text;
    /// Decode text from a double quoted text in the `<style>` element.
    decode_style_double_quoted_text;
    /// Write text from a double quoted text in the `<style>` element to a mutable `String` reference and return the encoded string slice.
    decode_style_double_quoted_text_to_string;
    /// Write text from a double quoted text in the `<style>` element to a mutable `Vec<u8>` reference and return the encoded data slice.
    decode_style_double_quoted_text_to_vec;
    /// Write text from a double quoted text in the `<style>` element to a writer.
    decode_style_double_quoted_text_to_writer;
}

decode_impl! {
    6;
    /// The following substring and characters are unescaped:
    ///
    /// * `<\/style>` => `</style>`
    /// * `\"` => `"`
    /// * `\'` => `'`
    parse_style_quoted_text;
    /// Decode text from a quoted text in the `<style>` element.
    decode_style_quoted_text;
    /// Write text from a quoted text in the `<style>` element to a mutable `String` reference and return the encoded string slice.
    decode_style_quoted_text_to_string;
    /// Write text from a quoted text in the `<style>` element to a mutable `Vec<u8>` reference and return the encoded data slice.
    decode_style_quoted_text_to_vec;
    /// Write text from a quoted text in the `<style>` element to a writer.
    decode_style_quoted_text_to_writer;
}
