use ethers_core::types::{Address, Bytes, I256, U256};
use foundry_common::fmt::UIfmt;

/// A format specifier.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FormatSpec {
    /// %s format spec
    #[default]
    String,
    /// %d format spec
    Number,
    /// %i format spec
    Integer,
    /// %o format spec
    Object,
}

impl FormatSpec {
    fn from_char(ch: char) -> Option<Self> {
        match ch {
            's' => Some(Self::String),
            'd' => Some(Self::Number),
            'i' => Some(Self::Integer),
            'o' => Some(Self::Object),
            _ => None,
        }
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
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl ConsoleFmt for bool {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number => (*self as i32).to_string(),
            FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl ConsoleFmt for U256 {
    fn fmt(&self, _spec: FormatSpec) -> String {
        self.pretty()
    }
}

impl ConsoleFmt for I256 {
    fn fmt(&self, _spec: FormatSpec) -> String {
        self.pretty()
    }
}

impl ConsoleFmt for Address {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl ConsoleFmt for Bytes {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl<const N: usize> ConsoleFmt for [u8; N] {
    fn fmt(&self, _spec: FormatSpec) -> String {
        self.pretty()
    }
}

/// Formats a string using the input values.
///
/// Formatting rules are the same as Hardhat. The supported format specifiers are as follows:
/// - %s: Converts the value using its String representation. This is equivalent to applying
///   [`UIfmt::pretty()`] on the format string.
/// - %d, %i: Converts the value to an integer. If a non-numeric value, such as String or Address,
///   is passed, then the spec is formatted as `NaN`.
/// - %o: Treats the format value as a javascript "object" and converts it to its string
///   representation.
/// - %%: This is parsed as a single percent sign ('%') without consuming any input value.
///
/// Unformatted values are appended to the end of the formatted output using [`UIfmt::pretty()`].
/// If there are more format specifiers than values, then the remaining unparsed format specifiers
/// appended to the formatted output as-is.
///
/// # Example
///
/// ```ignore
/// let formatted = console_format("%s has %d characters", ["foo", 3]);
/// assert_eq!(formatted, "foo has 3 characters");
/// ```
pub fn console_format<'a>(
    spec: &str,
    values: impl IntoIterator<Item = &'a dyn ConsoleFmt>,
) -> String {
    let mut values = values.into_iter();
    let mut result = String::with_capacity(spec.len());

    // for the first space
    let mut write_space = true;
    let last_value = if spec.is_empty() {
        // we still want to print any remaining values
        write_space = false;
        values.next()
    } else {
        format_spec(spec, &mut values, &mut result)
    };

    // append any remaining values with the standard format
    if let Some(v) = last_value {
        for v in std::iter::once(v).chain(values) {
            let fmt = v.fmt(FormatSpec::String);
            if write_space {
                result.push(' ');
            }
            result.push_str(&fmt);
            write_space = true;
        }
    }

    result
}

fn format_spec<'a>(
    s: &str,
    values: &mut impl Iterator<Item = &'a dyn ConsoleFmt>,
    result: &mut String,
) -> Option<&'a dyn ConsoleFmt> {
    let mut expect_fmt = false;
    let mut current_value = values.next();

    for (i, ch) in s.char_indices() {
        // no more values
        if current_value.is_none() {
            result.push_str(&s[i..].replace("%%", "%"));
            break
        }

        if expect_fmt {
            expect_fmt = false;
            if let Some(spec) = FormatSpec::from_char(ch) {
                // format and write the value
                let string = current_value.unwrap().fmt(spec);
                result.push_str(&string);
                current_value = values.next();
            } else {
                // invalid specifier or a second `%`, in both cases we ignore
                result.push(ch);
            }
        } else {
            expect_fmt = ch == '%';
            // push when not a `%` or it's the last char
            if !expect_fmt || i == s.len() - 1 {
                result.push(ch);
            }
        }
    }

    current_value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ConsoleFmt;
    use std::str::FromStr;

    macro_rules! logf1 {
        ($a:ident) => {{
            let args: [&dyn ConsoleFmt; 1] = [&$a.p_1];
            console_format(&$a.p_0, args)
        }};
    }

    macro_rules! logf2 {
        ($a:ident) => {{
            let args: [&dyn ConsoleFmt; 2] = [&$a.p_1, &$a.p_2];
            console_format(&$a.p_0, args)
        }};
    }

    macro_rules! logf3 {
        ($a:ident) => {{
            let args: [&dyn ConsoleFmt; 3] = [&$a.p_1, &$a.p_2, &$a.p_3];
            console_format(&$a.p_0, args)
        }};
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
        let console_log_format_1 = |spec: &str, arg: &dyn ConsoleFmt| {
            let args: [&dyn ConsoleFmt; 1] = [arg];
            console_format(spec, args)
        };

        assert_eq!("foo", console_log_format_1("%s", &String::from("foo")));
        assert_eq!("NaN", console_log_format_1("%i", &String::from("foo")));
        assert_eq!("'foo'", console_log_format_1("%o", &String::from("foo")));
        assert_eq!("%s foo", console_log_format_1("%%s", &String::from("foo")));
        assert_eq!("% foo", console_log_format_1("%", &String::from("foo")));
        assert_eq!("% foo", console_log_format_1("%%", &String::from("foo")));

        assert_eq!("true", console_log_format_1("%s", &true));
        assert_eq!("1", console_log_format_1("%d", &true));
        assert_eq!("0", console_log_format_1("%d", &false));
        assert_eq!("NaN", console_log_format_1("%i", &true));
        assert_eq!("'true'", console_log_format_1("%o", &true));

        let addr = Address::from_str("0xdEADBEeF00000000000000000000000000000000").unwrap();
        assert_eq!("0xdEADBEeF00000000000000000000000000000000", console_log_format_1("%s", &addr));
        assert_eq!("NaN", console_log_format_1("%d", &addr));
        assert_eq!("NaN", console_log_format_1("%i", &addr));
        assert_eq!(
            "'0xdEADBEeF00000000000000000000000000000000'",
            console_log_format_1("%o", &addr)
        );

        let bytes = Bytes::from_str("0xdeadbeef").unwrap();
        assert_eq!("0xdeadbeef", console_log_format_1("%s", &bytes));
        assert_eq!("NaN", console_log_format_1("%d", &bytes));
        assert_eq!("NaN", console_log_format_1("%i", &bytes));
        assert_eq!("'0xdeadbeef'", console_log_format_1("%o", &bytes));

        assert_eq!("100", console_log_format_1("%s", &U256::from(100)));
        assert_eq!("100", console_log_format_1("%d", &U256::from(100)));
        assert_eq!("100", console_log_format_1("%i", &U256::from(100)));
        assert_eq!("100", console_log_format_1("%o", &U256::from(100)));

        assert_eq!("100", console_log_format_1("%s", &I256::from(100)));
        assert_eq!("100", console_log_format_1("%d", &I256::from(100)));
        assert_eq!("100", console_log_format_1("%i", &I256::from(100)));
        assert_eq!("100", console_log_format_1("%o", &I256::from(100)));
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
    }

    #[test]
    fn test_derive_format() {
        let log1 = Log1 { p_0: String::from("foo %s bar"), p_1: U256::from(42) };
        assert_eq!(log1.fmt(Default::default()), "foo 42 bar");
        let call = Logs::Log1(log1);
        assert_eq!(call.fmt(Default::default()), "foo 42 bar");
    }
}
