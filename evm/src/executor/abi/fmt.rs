use super::HardhatConsoleCalls;
use ethers::types::{Address, Bytes, I256, U256};
use foundry_common::fmt::UIfmt;
use std::fmt::Write;

/// Represents a format specifier
#[derive(Debug)]
enum FormatSpec {
    /// %s format spec
    String,
    /// %d format spec
    Number,
    /// %i format spec
    Integer,
    /// %o format spec
    Object,
}

/// FormatValue specifies how a value type is to be formatted
trait FormatValue: UIfmt {
    /// Formats a value according to the FormatSpec
    fn fmt(&self, spec: FormatSpec) -> String;
}

impl FormatValue for String {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.clone(),
            FormatSpec::Object => format!("'{}'", self.clone()),
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl FormatValue for bool {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number => (*self as i32).to_string(),
            FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl FormatValue for U256 {
    fn fmt(&self, _spec: FormatSpec) -> String {
        self.pretty()
    }
}

impl FormatValue for I256 {
    fn fmt(&self, _spec: FormatSpec) -> String {
        self.pretty()
    }
}

impl FormatValue for Address {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

impl FormatValue for Bytes {
    fn fmt(&self, spec: FormatSpec) -> String {
        match spec {
            FormatSpec::String => self.pretty(),
            FormatSpec::Object => format!("'{}'", self.pretty()),
            FormatSpec::Number | FormatSpec::Integer => String::from("NaN"),
        }
    }
}

/// Formats a `specstr` using the input values, v1, v2 and v3.
/// For example:
///   console_log_format_n("%s has %d characters", 2, "foo", 3) == "foo has 3 characters"
/// num_v determines the maximum number of values to be used for the formatting (since we don't have
/// variadics in Rust). For example: console_log_format_n("%s", 1, v1, v2, v3) will format the
/// string using v1 only, ignoring v2 and v2.
///
/// Formatting rules are the same as hardhat. The supported format specifiers are as follows:
/// - %s: Converts the value using its String representation. This is equivalent to applying
///   UIfmt::pretty() on the format string.
/// - %d, %i: Converts the value to an integer. If a non-numeric value, such as String or Address,
///   is passed, then the spec is formatted as `NaN`.
/// - %o: Treats the format value as a javascript "object" and converts it to its string
///   representation.
/// - %%: This is parsed as a single percent sign ('%') without consuming any input value.
///
/// Unformatted values are appended to the end of the formatted output using UIfmt::pretty().
/// If there are more format specifiers than values, then the remaining unparsed format specifiers
/// appended to the formatted output as-is.
fn console_log_format_n(
    specstr: &str,
    num_v: isize,
    v0: &dyn FormatValue,
    v1: &dyn FormatValue,
    v2: &dyn FormatValue,
) -> String {
    assert!(num_v <= 3);

    let mut result = String::new();
    let spec = specstr.as_bytes();
    let mut expect_fmt = false;
    let mut curr_value = 0;
    for (pos, c) in spec.iter().enumerate() {
        if curr_value >= num_v {
            let suffix = String::from_utf8_lossy(&spec[pos..]);
            result.push_str(&suffix.replace("%%", "%"));
            break
        }

        result.push(*c as char);

        if expect_fmt && (*c == b's' || *c == b'd' || *c == b'i' || *c == b'o') {
            expect_fmt = false;
            // remove the 2 char fmt specifier
            result.pop();
            result.pop();
            let fspec = match *c {
                b's' => FormatSpec::String,
                b'd' => FormatSpec::Number,
                b'i' => FormatSpec::Integer,
                b'o' => FormatSpec::Object,
                _ => unreachable!(),
            };
            match curr_value {
                0 => result.push_str(&v0.fmt(fspec)),
                1 => result.push_str(&v1.fmt(fspec)),
                2 => result.push_str(&v2.fmt(fspec)),
                _ => {} // unreacheable
            };
            curr_value += 1;
        }

        if *c == b'%' {
            if pos == 0 {
                expect_fmt = true;
            } else {
                expect_fmt = spec[pos - 1] != b'%';
                if !expect_fmt {
                    result.pop(); // escape observed %%
                }
            }
        }
    }

    match num_v {
        1 => {
            if curr_value == 0 {
                write!(result, " {}", v0.pretty()).unwrap()
            }
        }
        2 => match curr_value {
            0 => write!(result, " {} {}", v0.pretty(), v1.pretty()).unwrap(),
            1 => write!(result, " {}", v1.pretty()).unwrap(),
            _ => {}
        },
        3 => match curr_value {
            0 => write!(result, " {} {} {}", v0.pretty(), v1.pretty(), v2.pretty()).unwrap(),
            1 => write!(result, " {} {}", v1.pretty(), v2.pretty()).unwrap(),
            2 => write!(result, " {}", v2.pretty()).unwrap(),
            _ => {}
        },
        _ => {}
    }
    result
}

fn console_log_format_1(specstr: &str, v0: &dyn FormatValue) -> String {
    let placeholder = String::new();
    console_log_format_n(specstr, 1, v0, &placeholder, &placeholder)
}

fn console_log_format_2(specstr: &str, v0: &dyn FormatValue, v1: &dyn FormatValue) -> String {
    let placeholder = String::new();
    console_log_format_n(specstr, 2, v0, v1, &placeholder)
}

fn console_log_format_3(
    specstr: &str,
    v0: &dyn FormatValue,
    v1: &dyn FormatValue,
    v2: &dyn FormatValue,
) -> String {
    console_log_format_n(specstr, 3, v0, v1, v2)
}

macro_rules! logf1 {
    ($a:ident) => {
        console_log_format_1(&$a.p_0, &$a.p_1)
    };
}
macro_rules! logf2 {
    ($a:ident) => {
        console_log_format_2(&$a.p_0, &$a.p_1, &$a.p_2)
    };
}
macro_rules! logf3 {
    ($a:ident) => {
        console_log_format_3(&$a.p_0, &$a.p_1, &$a.p_2, &$a.p_3)
    };
}

/// Formats a console.log call into a String. See [`console_log_format_n`] for details on the
/// formatting rules
pub fn format_hardhat_call(call: &HardhatConsoleCalls) -> String {
    match call {
        HardhatConsoleCalls::Log7(c) => logf1!(c),
        HardhatConsoleCalls::Log9(c) => logf1!(c),
        HardhatConsoleCalls::Log17(c) => logf1!(c),
        HardhatConsoleCalls::Log18(c) => logf1!(c),

        HardhatConsoleCalls::Log24(c) => logf2!(c),
        HardhatConsoleCalls::Log30(c) => logf2!(c),
        HardhatConsoleCalls::Log35(c) => logf2!(c),
        HardhatConsoleCalls::Log42(c) => logf2!(c),
        HardhatConsoleCalls::Log43(c) => logf2!(c),
        HardhatConsoleCalls::Log53(c) => logf2!(c),
        HardhatConsoleCalls::Log55(c) => logf2!(c),
        HardhatConsoleCalls::Log57(c) => logf2!(c),
        HardhatConsoleCalls::Log62(c) => logf2!(c),
        HardhatConsoleCalls::Log67(c) => logf2!(c),
        HardhatConsoleCalls::Log68(c) => logf2!(c),
        HardhatConsoleCalls::Log69(c) => logf2!(c),
        HardhatConsoleCalls::Log70(c) => logf2!(c),
        HardhatConsoleCalls::Log76(c) => logf2!(c),
        HardhatConsoleCalls::Log77(c) => logf2!(c),
        HardhatConsoleCalls::Log84(c) => logf2!(c),

        HardhatConsoleCalls::Log87(c) => logf3!(c),
        HardhatConsoleCalls::Log100(c) => logf3!(c),
        HardhatConsoleCalls::Log123(c) => logf3!(c),
        HardhatConsoleCalls::Log125(c) => logf3!(c),
        HardhatConsoleCalls::Log127(c) => logf3!(c),
        HardhatConsoleCalls::Log133(c) => logf3!(c),
        HardhatConsoleCalls::Log136(c) => logf3!(c),
        HardhatConsoleCalls::Log138(c) => logf3!(c),
        HardhatConsoleCalls::Log140(c) => logf3!(c),
        HardhatConsoleCalls::Log149(c) => logf3!(c),
        HardhatConsoleCalls::Log150(c) => logf3!(c),
        HardhatConsoleCalls::Log151(c) => logf3!(c),
        HardhatConsoleCalls::Log153(c) => logf3!(c),
        HardhatConsoleCalls::Log165(c) => logf3!(c),
        HardhatConsoleCalls::Log173(c) => logf3!(c),
        HardhatConsoleCalls::Log174(c) => logf3!(c),
        HardhatConsoleCalls::Log177(c) => logf3!(c),
        HardhatConsoleCalls::Log179(c) => logf3!(c),
        HardhatConsoleCalls::Log180(c) => logf3!(c),
        HardhatConsoleCalls::Log182(c) => logf3!(c),
        HardhatConsoleCalls::Log183(c) => logf3!(c),
        HardhatConsoleCalls::Log184(c) => logf3!(c),
        HardhatConsoleCalls::Log190(c) => logf3!(c),
        HardhatConsoleCalls::Log191(c) => logf3!(c),
        HardhatConsoleCalls::Log203(c) => logf3!(c),
        HardhatConsoleCalls::Log208(c) => logf3!(c),
        HardhatConsoleCalls::Log210(c) => logf3!(c),
        HardhatConsoleCalls::Log212(c) => logf3!(c),
        HardhatConsoleCalls::Log213(c) => logf3!(c),
        HardhatConsoleCalls::Log217(c) => logf3!(c),
        HardhatConsoleCalls::Log218(c) => logf3!(c),
        HardhatConsoleCalls::Log219(c) => logf3!(c),
        HardhatConsoleCalls::Log222(c) => logf3!(c),
        HardhatConsoleCalls::Log224(c) => logf3!(c),
        HardhatConsoleCalls::Log226(c) => logf3!(c),
        HardhatConsoleCalls::Log230(c) => logf3!(c),
        HardhatConsoleCalls::Log231(c) => logf3!(c),
        HardhatConsoleCalls::Log235(c) => logf3!(c),
        HardhatConsoleCalls::Log237(c) => logf3!(c),
        HardhatConsoleCalls::Log238(c) => logf3!(c),
        HardhatConsoleCalls::Log244(c) => logf3!(c),
        HardhatConsoleCalls::Log245(c) => logf3!(c),
        HardhatConsoleCalls::Log247(c) => logf3!(c),
        HardhatConsoleCalls::Log254(c) => logf3!(c),
        HardhatConsoleCalls::Log256(c) => logf3!(c),
        HardhatConsoleCalls::Log267(c) => logf3!(c),
        HardhatConsoleCalls::Log268(c) => logf3!(c),
        HardhatConsoleCalls::Log270(c) => logf3!(c),
        HardhatConsoleCalls::Log272(c) => logf3!(c),
        HardhatConsoleCalls::Log278(c) => logf3!(c),
        HardhatConsoleCalls::Log289(c) => logf3!(c),
        HardhatConsoleCalls::Log290(c) => logf3!(c),
        HardhatConsoleCalls::Log294(c) => logf3!(c),
        HardhatConsoleCalls::Log306(c) => logf3!(c),
        HardhatConsoleCalls::Log312(c) => logf3!(c),
        HardhatConsoleCalls::Log314(c) => logf3!(c),
        HardhatConsoleCalls::Log315(c) => logf3!(c),
        HardhatConsoleCalls::Log316(c) => logf3!(c),
        HardhatConsoleCalls::Log320(c) => logf3!(c),
        HardhatConsoleCalls::Log323(c) => logf3!(c),
        HardhatConsoleCalls::Log326(c) => logf3!(c),
        HardhatConsoleCalls::Log330(c) => logf3!(c),
        HardhatConsoleCalls::Log335(c) => logf3!(c),
        HardhatConsoleCalls::Log338(c) => logf3!(c),
        _ => call.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::abi::*;

    #[test]
    fn test_console_log_format_specifiers() {
        use std::str::FromStr;

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
        use std::str::FromStr;

        let mut log17call = Log17Call { p_0: "foo %s".to_string(), p_1: U256::from(100) };
        assert_eq!("foo 100", logf1!(log17call));
        log17call.p_0 = String::from("foo");
        assert_eq!("foo 100", logf1!(log17call));
        log17call.p_0 = String::from("%s foo");
        assert_eq!("100 foo", logf1!(log17call));

        let mut log68call =
            Log68Call { p_0: "foo %s %s".to_string(), p_1: true, p_2: U256::from(100) };
        assert_eq!("foo true 100", logf2!(log68call));
        log68call.p_0 = String::from("foo");
        assert_eq!("foo true 100", logf2!(log68call));
        log68call.p_0 = String::from("%s %s foo");
        assert_eq!("true 100 foo", logf2!(log68call));

        let log149call = Log149Call {
            p_0: String::from("foo %s %%s %s and %d foo %%"),
            p_1: Address::from_str("0xdEADBEeF00000000000000000000000000000000").unwrap(),
            p_2: true,
            p_3: U256::from(21),
        };
        assert_eq!(
            "foo 0xdEADBEeF00000000000000000000000000000000 %s true and 21 foo %",
            logf3!(log149call)
        );
    }
}
