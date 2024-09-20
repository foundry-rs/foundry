use super::UIfmt;
use alloy_primitives::{Address, Bytes, FixedBytes, I256, U256};
use std::fmt::{self, Write};

/// A piece is a portion of the format string which represents the next part to emit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece<'a> {
    /// A literal string which should directly be emitted.
    String(&'a str),
    /// A format specifier which should be replaced with the next argument.
    NextArgument(FormatSpec),
}

/// A format specifier.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FormatSpec {
    /// `%s`
    #[default]
    String,
    /// `%d`
    Number,
    /// `%i`
    Integer,
    /// `%o`
    Object,
    /// `%e`, `%18e`
    Exponential(Option<usize>),
    /// `%x`
    Hexadecimal,
}

impl fmt::Display for FormatSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("%")?;
        match *self {
            Self::String => f.write_str("s"),
            Self::Number => f.write_str("d"),
            Self::Integer => f.write_str("i"),
            Self::Object => f.write_str("o"),
            Self::Exponential(Some(n)) => write!(f, "{n}e"),
            Self::Exponential(None) => f.write_str("e"),
            Self::Hexadecimal => f.write_str("x"),
        }
    }
}

enum ParseArgError {
    /// Failed to parse the argument.
    Err,
    /// Escape `%%`.
    Skip,
}

/// Parses a format string into a sequence of [pieces][Piece].
#[derive(Debug)]
pub struct Parser<'a> {
    input: &'a str,
    chars: std::str::CharIndices<'a>,
}

impl<'a> Parser<'a> {
    /// Creates a new parser for the given input.
    pub fn new(input: &'a str) -> Self {
        Self { input, chars: input.char_indices() }
    }

    /// Parses a string until the next format specifier.
    ///
    /// `skip` is the number of format specifier characters (`%`) to ignore before returning the
    /// string.
    fn string(&mut self, start: usize, mut skip: usize) -> &'a str {
        while let Some((pos, c)) = self.peek() {
            if c == '%' {
                if skip == 0 {
                    return &self.input[start..pos];
                }
                skip -= 1;
            }
            self.chars.next();
        }
        &self.input[start..]
    }

    /// Parses a format specifier.
    ///
    /// If `Err` is returned, the internal iterator may have been advanced and it may be in an
    /// invalid state.
    fn argument(&mut self) -> Result<FormatSpec, ParseArgError> {
        let (start, ch) = self.peek().ok_or(ParseArgError::Err)?;
        let simple_spec = match ch {
            's' => Some(FormatSpec::String),
            'd' => Some(FormatSpec::Number),
            'i' => Some(FormatSpec::Integer),
            'o' => Some(FormatSpec::Object),
            'e' => Some(FormatSpec::Exponential(None)),
            'x' => Some(FormatSpec::Hexadecimal),
            // "%%" is a literal '%'.
            '%' => return Err(ParseArgError::Skip),
            _ => None,
        };
        if let Some(spec) = simple_spec {
            self.chars.next();
            return Ok(spec);
        }

        // %<n>e
        if ch.is_ascii_digit() {
            let n = self.integer(start);
            if let Some((_, 'e')) = self.peek() {
                self.chars.next();
                return Ok(FormatSpec::Exponential(n));
            }
        }

        Err(ParseArgError::Err)
    }

    fn integer(&mut self, start: usize) -> Option<usize> {
        let mut end = start;
        while let Some((pos, ch)) = self.peek() {
            if !ch.is_ascii_digit() {
                end = pos;
                break;
            }
            self.chars.next();
        }
        self.input[start..end].parse().ok()
    }

    fn current_pos(&mut self) -> usize {
        self.peek().map(|(n, _)| n).unwrap_or(self.input.len())
    }

    fn peek(&mut self) -> Option<(usize, char)> {
        self.peek_n(0)
    }

    fn peek_n(&mut self, n: usize) -> Option<(usize, char)> {
        self.chars.clone().nth(n)
    }
}

impl<'a> Iterator for Parser<'a> {
    type Item = Piece<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        let (mut start, ch) = self.peek()?;
        let mut skip = 0;
        if ch == '%' {
            let prev = self.chars.clone();
            self.chars.next();
            match self.argument() {
                Ok(arg) => {
                    debug_assert_eq!(arg.to_string(), self.input[start..self.current_pos()]);
                    return Some(Piece::NextArgument(arg));
                }

                // Skip the argument if we encountered "%%".
                Err(ParseArgError::Skip) => {
                    start = self.current_pos();
                    skip += 1;
                }

                // Reset the iterator if we failed to parse the argument, and include any
                // parsed and unparsed specifier in `String`.
                Err(ParseArgError::Err) => {
                    self.chars = prev;
                    skip += 1;
                }
            }
        }
        Some(Piece::String(self.string(start, skip)))
    }
}

/// Formats a value using a [FormatSpec].
pub trait ConsoleFmt {
    /// Formats a value using a [FormatSpec].
    fn fmt(&self, spec: FormatSpec) -> String;
}

impl ConsoleFmt for String {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.clone(),
            FormatSpec::Object => format!("'{}'", self.clone()),
            FormatSpec::Number |
            FormatSpec::Integer |
            FormatSpec::Exponential(_) |
            FormatSpec::Hexadecimal => Self::from("NaN"),
        }
    }
}

impl ConsoleFmt for bool {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number => (*self as i32).to_string(),
            FormatSpec::Integer | FormatSpec::Exponential(_) | FormatSpec::Hexadecimal => {
                String::from("NaN")
            }
        }
    }
}

impl ConsoleFmt for U256 {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String | FormatSpec::Object | FormatSpec::Number | FormatSpec::Integer => {
                self.pretty()
            }
            FormatSpec::Hexadecimal => {
                let hex = format!("{self:x}");
                format!("0x{}", hex.trim_start_matches('0'))
            }
            FormatSpec::Exponential(None) => {
                let log = self.pretty().len() - 1;
                let exp10 = Self::from(10).pow(Self::from(log));
                let amount = *self;
                let integer = amount / exp10;
                let decimal = (amount % exp10).to_string();
                let decimal = format!("{decimal:0>log$}").trim_end_matches('0').to_string();
                if !decimal.is_empty() {
                    format!("{integer}.{decimal}e{log}")
                } else {
                    format!("{integer}e{log}")
                }
            }
            FormatSpec::Exponential(Some(precision)) => {
                let exp10 = Self::from(10).pow(Self::from(precision));
                let amount = *self;
                let integer = amount / exp10;
                let decimal = (amount % exp10).to_string();
                let decimal = format!("{decimal:0>precision$}").trim_end_matches('0').to_string();
                if !decimal.is_empty() {
                    format!("{integer}.{decimal}")
                } else {
                    format!("{integer}")
                }
            }
        }
    }
}

impl ConsoleFmt for I256 {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String | FormatSpec::Object | FormatSpec::Number | FormatSpec::Integer => {
                self.pretty()
            }
            FormatSpec::Hexadecimal => {
                let hex = format!("{self:x}");
                format!("0x{}", hex.trim_start_matches('0'))
            }
            FormatSpec::Exponential(None) => {
                let amount = *self;
                let sign = if amount.is_negative() { "-" } else { "" };
                let log = if amount.is_negative() {
                    self.pretty().len() - 2
                } else {
                    self.pretty().len() - 1
                };
                let exp10 = Self::exp10(log);
                let integer = (amount / exp10).twos_complement();
                let decimal = (amount % exp10).twos_complement().to_string();
                let decimal = format!("{decimal:0>log$}").trim_end_matches('0').to_string();
                if !decimal.is_empty() {
                    format!("{sign}{integer}.{decimal}e{log}")
                } else {
                    format!("{sign}{integer}e{log}")
                }
            }
            FormatSpec::Exponential(Some(precision)) => {
                let amount = *self;
                let sign = if amount.is_negative() { "-" } else { "" };
                let exp10 = Self::exp10(precision);
                let integer = (amount / exp10).twos_complement();
                let decimal = (amount % exp10).twos_complement().to_string();
                let decimal = format!("{decimal:0>precision$}").trim_end_matches('0').to_string();
                if !decimal.is_empty() {
                    format!("{sign}{integer}.{decimal}")
                } else {
                    format!("{sign}{integer}")
                }
            }
        }
    }
}

impl ConsoleFmt for Address {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String | FormatSpec::Hexadecimal => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer | FormatSpec::Exponential(_) => {
                String::from("NaN")
            }
        }
    }
}

impl ConsoleFmt for Vec<u8> {
    fn fmt(&self, spec: FormatSpec) -> String {
        self[..].fmt(spec)
    }
}

impl ConsoleFmt for Bytes {
    fn fmt(&self, spec: FormatSpec) -> String {
        self[..].fmt(spec)
    }
}

impl<const N: usize> ConsoleFmt for [u8; N] {
    fn fmt(&self, spec: FormatSpec) -> String {
        self[..].fmt(spec)
    }
}

impl<const N: usize> ConsoleFmt for FixedBytes<N> {
    fn fmt(&self, spec: FormatSpec) -> String {
        self[..].fmt(spec)
    }
}

impl ConsoleFmt for [u8] {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String | FormatSpec::Hexadecimal => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer | FormatSpec::Exponential(_) => {
                String::from("NaN")
            }
        }
    }
}

/// Formats a string using the input values.
///
/// Formatting rules are the same as Hardhat. The supported format specifiers are as follows:
/// - %s: Converts the value using its String representation. This is equivalent to applying
///   [`UIfmt::pretty()`] on the format string.
/// - %o: Treats the format value as a javascript "object" and converts it to its string
///   representation.
/// - %d, %i: Converts the value to an integer. If a non-numeric value, such as String or Address,
///   is passed, then the spec is formatted as `NaN`.
/// - %x: Converts the value to a hexadecimal string. If a non-numeric value, such as String or
///   Address, is passed, then the spec is formatted as `NaN`.
/// - %e: Converts the value to an exponential notation string. If a non-numeric value, such as
///   String or Address, is passed, then the spec is formatted as `NaN`.
/// - %%: This is parsed as a single percent sign ('%') without consuming any input value.
///
/// Unformatted values are appended to the end of the formatted output using [`UIfmt::pretty()`].
/// If there are more format specifiers than values, then the remaining unparsed format specifiers
/// appended to the formatted output as-is.
///
/// # Examples
///
/// ```ignore (not implemented for integers)
/// let formatted = foundry_common::fmt::console_format("%s has %d characters", &[&"foo", &3]);
/// assert_eq!(formatted, "foo has 3 characters");
/// ```
pub fn console_format(spec: &str, values: &[&dyn ConsoleFmt]) -> String {
    let mut values = values.iter().copied();
    let mut result = String::with_capacity(spec.len());

    // for the first space
    let mut write_space = if spec.is_empty() {
        false
    } else {
        format_spec(spec, &mut values, &mut result);
        true
    };

    // append any remaining values with the standard format
    for v in values {
        let fmt = v.fmt(FormatSpec::String);
        if write_space {
            result.push(' ');
        }
        result.push_str(&fmt);
        write_space = true;
    }

    result
}

fn format_spec<'a>(
    s: &str,
    mut values: impl Iterator<Item = &'a dyn ConsoleFmt>,
    result: &mut String,
) {
    for piece in Parser::new(s) {
        match piece {
            Piece::String(s) => result.push_str(s),
            Piece::NextArgument(spec) => {
                if let Some(value) = values.next() {
                    result.push_str(&value.fmt(spec));
                } else {
                    // Write the format specifier as-is if there are no more values.
                    write!(result, "{spec}").unwrap();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::B256;
    use foundry_macros::ConsoleFmt;
    use std::str::FromStr;

    macro_rules! logf1 {
        ($a:ident) => {
            console_format(&$a.p_0, &[&$a.p_1])
        };
    }

    macro_rules! logf2 {
        ($a:ident) => {
            console_format(&$a.p_0, &[&$a.p_1, &$a.p_2])
        };
    }

    macro_rules! logf3 {
        ($a:ident) => {
            console_format(&$a.p_0, &[&$a.p_1, &$a.p_2, &$a.p_3])
        };
    }

    #[derive(Clone, Debug, ConsoleFmt)]
    struct Log1 {
        p_0: String,
        p_1: U256,
    }

    #[derive(Clone, Debug, ConsoleFmt)]
    struct Log2 {
        p_0: String,
        p_1: bool,
        p_2: U256,
    }

    #[derive(Clone, Debug, ConsoleFmt)]
    struct Log3 {
        p_0: String,
        p_1: Address,
        p_2: bool,
        p_3: U256,
    }

    #[allow(unused)]
    #[derive(Clone, Debug, ConsoleFmt)]
    enum Logs {
        Log1(Log1),
        Log2(Log2),
        Log3(Log3),
    }

    #[test]
    fn test_console_log_format_specifiers() {
        let fmt_1 = |spec: &str, arg: &dyn ConsoleFmt| console_format(spec, &[arg]);

        assert_eq!("foo", fmt_1("%s", &String::from("foo")));
        assert_eq!("NaN", fmt_1("%d", &String::from("foo")));
        assert_eq!("NaN", fmt_1("%i", &String::from("foo")));
        assert_eq!("NaN", fmt_1("%e", &String::from("foo")));
        assert_eq!("NaN", fmt_1("%x", &String::from("foo")));
        assert_eq!("'foo'", fmt_1("%o", &String::from("foo")));
        assert_eq!("%s foo", fmt_1("%%s", &String::from("foo")));
        assert_eq!("% foo", fmt_1("%", &String::from("foo")));
        assert_eq!("% foo", fmt_1("%%", &String::from("foo")));

        assert_eq!("true", fmt_1("%s", &true));
        assert_eq!("1", fmt_1("%d", &true));
        assert_eq!("0", fmt_1("%d", &false));
        assert_eq!("NaN", fmt_1("%i", &true));
        assert_eq!("NaN", fmt_1("%e", &true));
        assert_eq!("NaN", fmt_1("%x", &true));
        assert_eq!("'true'", fmt_1("%o", &true));

        let b32 =
            B256::from_str("0xdeadbeef00000000000000000000000000000000000000000000000000000000")
                .unwrap();
        assert_eq!(
            "0xdeadbeef00000000000000000000000000000000000000000000000000000000",
            fmt_1("%s", &b32)
        );
        assert_eq!(
            "0xdeadbeef00000000000000000000000000000000000000000000000000000000",
            fmt_1("%x", &b32)
        );
        assert_eq!("NaN", fmt_1("%d", &b32));
        assert_eq!("NaN", fmt_1("%i", &b32));
        assert_eq!("NaN", fmt_1("%e", &b32));
        assert_eq!(
            "'0xdeadbeef00000000000000000000000000000000000000000000000000000000'",
            fmt_1("%o", &b32)
        );

        let addr = Address::from_str("0xdEADBEeF00000000000000000000000000000000").unwrap();
        assert_eq!("0xdEADBEeF00000000000000000000000000000000", fmt_1("%s", &addr));
        assert_eq!("NaN", fmt_1("%d", &addr));
        assert_eq!("NaN", fmt_1("%i", &addr));
        assert_eq!("NaN", fmt_1("%e", &addr));
        assert_eq!("0xdEADBEeF00000000000000000000000000000000", fmt_1("%x", &addr));
        assert_eq!("'0xdEADBEeF00000000000000000000000000000000'", fmt_1("%o", &addr));

        let bytes = Bytes::from_str("0xdeadbeef").unwrap();
        assert_eq!("0xdeadbeef", fmt_1("%s", &bytes));
        assert_eq!("NaN", fmt_1("%d", &bytes));
        assert_eq!("NaN", fmt_1("%i", &bytes));
        assert_eq!("NaN", fmt_1("%e", &bytes));
        assert_eq!("0xdeadbeef", fmt_1("%x", &bytes));
        assert_eq!("'0xdeadbeef'", fmt_1("%o", &bytes));

        assert_eq!("100", fmt_1("%s", &U256::from(100)));
        assert_eq!("100", fmt_1("%d", &U256::from(100)));
        assert_eq!("100", fmt_1("%i", &U256::from(100)));
        assert_eq!("1e2", fmt_1("%e", &U256::from(100)));
        assert_eq!("1.0023e6", fmt_1("%e", &U256::from(1002300)));
        assert_eq!("1.23e5", fmt_1("%e", &U256::from(123000)));
        assert_eq!("0x64", fmt_1("%x", &U256::from(100)));
        assert_eq!("100", fmt_1("%o", &U256::from(100)));

        assert_eq!("100", fmt_1("%s", &I256::try_from(100).unwrap()));
        assert_eq!("100", fmt_1("%d", &I256::try_from(100).unwrap()));
        assert_eq!("100", fmt_1("%i", &I256::try_from(100).unwrap()));
        assert_eq!("1e2", fmt_1("%e", &I256::try_from(100).unwrap()));
        assert_eq!("-1e2", fmt_1("%e", &I256::try_from(-100).unwrap()));
        assert_eq!("-1.0023e6", fmt_1("%e", &I256::try_from(-1002300).unwrap()));
        assert_eq!("-1.23e5", fmt_1("%e", &I256::try_from(-123000).unwrap()));
        assert_eq!("1.0023e6", fmt_1("%e", &I256::try_from(1002300).unwrap()));
        assert_eq!("1.23e5", fmt_1("%e", &I256::try_from(123000).unwrap()));

        // %ne
        assert_eq!("10", fmt_1("%1e", &I256::try_from(100).unwrap()));
        assert_eq!("-1", fmt_1("%2e", &I256::try_from(-100).unwrap()));
        assert_eq!("123000", fmt_1("%0e", &I256::try_from(123000).unwrap()));
        assert_eq!("12300", fmt_1("%1e", &I256::try_from(123000).unwrap()));
        assert_eq!("0.0123", fmt_1("%7e", &I256::try_from(123000).unwrap()));
        assert_eq!("-0.0123", fmt_1("%7e", &I256::try_from(-123000).unwrap()));

        assert_eq!("0x64", fmt_1("%x", &I256::try_from(100).unwrap()));
        assert_eq!(
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff9c",
            fmt_1("%x", &I256::try_from(-100).unwrap())
        );
        assert_eq!(
            "0xffffffffffffffffffffffffffffffffffffffffffffffffffffffe8b7891800",
            fmt_1("%x", &I256::try_from(-100000000000i64).unwrap())
        );
        assert_eq!("100", fmt_1("%o", &I256::try_from(100).unwrap()));

        // make sure that %byte values are not consumed when there are no values
        assert_eq!("%333d%3e%5F", console_format("%333d%3e%5F", &[]));
        assert_eq!(
            "%5d123456.789%2f%3f%e1",
            console_format("%5d%3e%2f%3f%e1", &[&U256::from(123456789)])
        );
    }

    #[test]
    fn test_console_log_format() {
        let mut log1 = Log1 { p_0: "foo %s".to_string(), p_1: U256::from(100) };
        assert_eq!("foo 100", logf1!(log1));
        log1.p_0 = String::from("foo");
        assert_eq!("foo 100", logf1!(log1));
        log1.p_0 = String::from("%s foo");
        assert_eq!("100 foo", logf1!(log1));

        let mut log2 = Log2 { p_0: "foo %s %s".to_string(), p_1: true, p_2: U256::from(100) };
        assert_eq!("foo true 100", logf2!(log2));
        log2.p_0 = String::from("foo");
        assert_eq!("foo true 100", logf2!(log2));
        log2.p_0 = String::from("%s %s foo");
        assert_eq!("true 100 foo", logf2!(log2));

        let log3 = Log3 {
            p_0: String::from("foo %s %%s %s and %d foo %%"),
            p_1: Address::from_str("0xdEADBEeF00000000000000000000000000000000").unwrap(),
            p_2: true,
            p_3: U256::from(21),
        };
        assert_eq!(
            "foo 0xdEADBEeF00000000000000000000000000000000 %s true and 21 foo %",
            logf3!(log3)
        );

        // %ne
        let log4 = Log1 { p_0: String::from("%5e"), p_1: U256::from(123456789) };
        assert_eq!("1234.56789", logf1!(log4));

        let log5 = Log1 { p_0: String::from("foo %3e bar"), p_1: U256::from(123456789) };
        assert_eq!("foo 123456.789 bar", logf1!(log5));

        let log6 =
            Log2 { p_0: String::from("%e and %12e"), p_1: false, p_2: U256::from(123456789) };
        assert_eq!("NaN and 0.000123456789", logf2!(log6));
    }

    #[test]
    fn test_derive_format() {
        let log1 = Log1 { p_0: String::from("foo %s bar"), p_1: U256::from(42) };
        assert_eq!(log1.fmt(Default::default()), "foo 42 bar");
        let call = Logs::Log1(log1);
        assert_eq!(call.fmt(Default::default()), "foo 42 bar");
    }
}
