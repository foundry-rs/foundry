use super::serde_helpers;
use serde::{Deserialize, Serialize};
use std::{fmt, ops::Range, str::FromStr};
use yansi::{Color, Style};

const ARROW: &str = "-->";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceLocation {
    pub file: String,
    pub start: i32,
    pub end: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecondarySourceLocation {
    pub file: Option<String>,
    pub start: Option<i32>,
    pub end: Option<i32>,
    pub message: Option<String>,
}

/// The severity of the error.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Solc `Error`
    #[default]
    Error,
    /// Solc `Warning`
    Warning,
    /// Solc `Info`
    Info,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Severity {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "Error" | "error" => Ok(Self::Error),
            "Warning" | "warning" => Ok(Self::Warning),
            "Info" | "info" => Ok(Self::Info),
            s => Err(format!("Invalid severity: {s}")),
        }
    }
}

impl Severity {
    /// Returns `true` if the severity is `Error`.
    pub const fn is_error(&self) -> bool {
        matches!(self, Self::Error)
    }

    /// Returns `true` if the severity is `Warning`.
    pub const fn is_warning(&self) -> bool {
        matches!(self, Self::Warning)
    }

    /// Returns `true` if the severity is `Info`.
    pub const fn is_info(&self) -> bool {
        matches!(self, Self::Info)
    }

    /// Returns the string representation of the severity.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Info => "Info",
        }
    }

    /// Returns the color to format the severity with.
    pub const fn color(&self) -> Color {
        match self {
            Self::Error => Color::Red,
            Self::Warning => Color::Yellow,
            Self::Info => Color::White,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Error {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_location: Option<SourceLocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secondary_source_locations: Vec<SecondarySourceLocation>,
    pub r#type: String,
    pub component: String,
    pub severity: Severity,
    #[serde(default, with = "serde_helpers::display_from_str_opt")]
    pub error_code: Option<u64>,
    pub message: String,
    pub formatted_message: Option<String>,
}

impl Error {
    /// Returns `true` if the error is an error.
    pub const fn is_error(&self) -> bool {
        self.severity.is_error()
    }

    /// Returns `true` if the error is a warning.
    pub const fn is_warning(&self) -> bool {
        self.severity.is_warning()
    }

    /// Returns `true` if the error is an info.
    pub const fn is_info(&self) -> bool {
        self.severity.is_info()
    }
}

/// Tries to mimic Solidity's own error formatting.
///
/// <https://github.com/ethereum/solidity/blob/a297a687261a1c634551b1dac0e36d4573c19afe/liblangutil/SourceReferenceFormatter.cpp#L105>
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut short_msg = self.message.trim();
        let fmtd_msg = self.formatted_message.as_deref().unwrap_or("");

        if short_msg.is_empty() {
            // if the message is empty, try to extract the first line from the formatted message
            if let Some(first_line) = fmtd_msg.lines().next() {
                // this is something like `ParserError: <short_message>`
                if let Some((_, s)) = first_line.split_once(':') {
                    short_msg = s.trim_start();
                } else {
                    short_msg = first_line;
                }
            }
        }

        // Error (XXXX): Error Message
        styled(f, self.severity.color().bold(), |f| self.fmt_severity(f))?;
        fmt_msg(f, short_msg)?;

        let mut lines = fmtd_msg.lines();

        if let Some(l) = lines.clone().next() {
            if l.bytes().filter(|&b| b == b':').count() >= 3
                && (l.contains(['/', '\\']) || l.contains(".sol"))
            {
                // This is an old style error message, like:
                //     path/to/file:line:column: ErrorType: message
                // We want to display this as-is.
            } else {
                // Otherwise, assume that the messages are the same until we find a source
                // location.
                lines.next();
                while let Some(line) = lines.clone().next() {
                    if line.contains(ARROW) {
                        break;
                    }
                    lines.next();
                }
            }
        }

        // Format the main source location.
        fmt_source_location(f, &mut lines)?;

        // Format remaining lines as secondary locations.
        while let Some(line) = lines.next() {
            f.write_str("\n")?;

            if let Some((note, msg)) = line.split_once(':') {
                styled(f, Self::secondary_style(), |f| f.write_str(note))?;
                fmt_msg(f, msg)?;
            } else {
                f.write_str(line)?;
            }

            fmt_source_location(f, &mut lines)?;
        }

        Ok(())
    }
}

impl Error {
    /// The style of the diagnostic severity.
    pub fn error_style(&self) -> Style {
        self.severity.color().bold()
    }

    /// The style of the diagnostic message.
    pub fn message_style() -> Style {
        Color::White.bold()
    }

    /// The style of the secondary source location.
    pub fn secondary_style() -> Style {
        Color::Cyan.bold()
    }

    /// The style of the source location highlight.
    pub fn highlight_style() -> Style {
        Style::new().fg(Color::Yellow)
    }

    /// The style of the diagnostics.
    pub fn diag_style() -> Style {
        Color::Yellow.bold()
    }

    /// The style of the source location frame.
    pub fn frame_style() -> Style {
        Style::new().fg(Color::Blue)
    }

    /// Formats the diagnostic severity:
    ///
    /// ```text
    /// Error (XXXX)
    /// ```
    fn fmt_severity(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.severity.as_str())?;
        if let Some(code) = self.error_code {
            write!(f, " ({code})")?;
        }
        Ok(())
    }
}

/// Calls `fun` in between [`Style::fmt_prefix`] and [`Style::fmt_suffix`].
fn styled<F>(f: &mut fmt::Formatter<'_>, style: Style, fun: F) -> fmt::Result
where
    F: FnOnce(&mut fmt::Formatter<'_>) -> fmt::Result,
{
    let enabled = yansi::is_enabled();
    if enabled {
        style.fmt_prefix(f)?;
    }
    fun(f)?;
    if enabled {
        style.fmt_suffix(f)?;
    }
    Ok(())
}

/// Formats the diagnostic message.
fn fmt_msg(f: &mut fmt::Formatter<'_>, msg: &str) -> fmt::Result {
    styled(f, Error::message_style(), |f| {
        f.write_str(": ")?;
        f.write_str(msg.trim_start())
    })
}

/// Colors a Solidity source location:
///
/// ```text
/// --> /home/user/contract.sol:420:69:
///     |
/// 420 |       bad_code()
///     |                ^
/// ```
fn fmt_source_location(f: &mut fmt::Formatter<'_>, lines: &mut std::str::Lines<'_>) -> fmt::Result {
    // --> source
    if let Some(line) = lines.next() {
        f.write_str("\n")?;
        if let Some((left, loc)) = line.split_once(ARROW) {
            f.write_str(left)?;
            styled(f, Error::frame_style(), |f| f.write_str(ARROW))?;
            f.write_str(loc)?;
        } else {
            f.write_str(line)?;
        }
    }

    // get the next 3 lines
    let Some(line1) = lines.next() else {
        return Ok(());
    };
    let Some(line2) = lines.next() else {
        f.write_str("\n")?;
        f.write_str(line1)?;
        return Ok(());
    };
    let Some(line3) = lines.next() else {
        f.write_str("\n")?;
        f.write_str(line1)?;
        f.write_str("\n")?;
        f.write_str(line2)?;
        return Ok(());
    };

    // line 1, just a frame
    fmt_framed_location(f, line1, None)?;

    // line 2, frame and code; highlight the text based on line 3's carets
    let hl_start = line3.find('^');
    let highlight = hl_start.map(|start| {
        let end = if line3.contains("^ (") {
            // highlight the entire line because of "spans across multiple lines" diagnostic
            line2.len()
        } else if let Some(carets) = line3[start..].find(|c: char| c != '^') {
            // highlight the text that the carets point to
            start + carets
        } else {
            // the carets span the entire third line
            line3.len()
        }
        // bound in case carets span longer than the code they point to
        .min(line2.len());
        (start.min(end)..end, Error::highlight_style())
    });
    fmt_framed_location(f, line2, highlight)?;

    // line 3, frame and maybe highlight, this time till the end unconditionally
    let highlight = hl_start.map(|i| (i..line3.len(), Error::diag_style()));
    fmt_framed_location(f, line3, highlight)
}

/// Colors a single Solidity framed source location line. Part of [`fmt_source_location`].
fn fmt_framed_location(
    f: &mut fmt::Formatter<'_>,
    line: &str,
    highlight: Option<(Range<usize>, Style)>,
) -> fmt::Result {
    f.write_str("\n")?;

    if let Some((space_or_line_number, rest)) = line.split_once('|') {
        // if the potential frame is not just whitespace or numbers, don't color it
        if !space_or_line_number.chars().all(|c| c.is_whitespace() || c.is_numeric()) {
            return f.write_str(line);
        }

        styled(f, Error::frame_style(), |f| {
            f.write_str(space_or_line_number)?;
            f.write_str("|")
        })?;

        if let Some((range, style)) = highlight {
            let Range { start, end } = range;
            // Skip highlighting if the range is not valid unicode.
            if !line.is_char_boundary(start) || !line.is_char_boundary(end) {
                f.write_str(rest)
            } else {
                let rest_start = line.len() - rest.len();
                f.write_str(&line[rest_start..start])?;
                styled(f, style, |f| f.write_str(&line[range]))?;
                f.write_str(&line[end..])
            }
        } else {
            f.write_str(rest)
        }
    } else {
        f.write_str(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt_unicode() {
        let msg = "Invalid character in string. If you are trying to use Unicode characters, use a unicode\"...\" string literal.";
        let e = Error {
            source_location: Some(SourceLocation { file: "test/Counter.t.sol".into(), start: 418, end: 462 }),
            secondary_source_locations: vec![],
            r#type: "ParserError".into(),
            component: "general".into(),
            severity: Severity::Error,
            error_code: Some(8936),
            message: msg.into(),
            formatted_message: Some("ParserError: Invalid character in string. If you are trying to use Unicode characters, use a unicode\"...\" string literal.\n  --> test/Counter.t.sol:17:21:\n   |\n17 |         console.log(\"1. ownership set correctly as governance: ✓\");\n   |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n\n".into()),
        };
        let s = e.to_string();
        eprintln!("{s}");
        assert!(s.contains(msg), "\n{s}");
    }

    #[test]
    fn only_formatted() {
        let e = Error {
            source_location: Some(SourceLocation { file: "test/Counter.t.sol".into(), start: 418, end: 462 }),
            secondary_source_locations: vec![],
            r#type: "ParserError".into(),
            component: "general".into(),
            severity: Severity::Error,
            error_code: Some(8936),
            message: String::new(),
            formatted_message: Some("ParserError: Invalid character in string. If you are trying to use Unicode characters, use a unicode\"...\" string literal.\n  --> test/Counter.t.sol:17:21:\n   |\n17 |         console.log(\"1. ownership set correctly as governance: ✓\");\n   |                     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n\n".into()),
        };
        let s = e.to_string();
        eprintln!("{s}");
        assert!(s.contains("Invalid character in string"), "\n{s}");
    }

    #[test]
    fn solc_0_7() {
        let output = r#"{"errors":[{"component":"general","errorCode":"6594","formattedMessage":"test/Counter.t.sol:7:1: TypeError: Contract \"CounterTest\" does not use ABI coder v2 but wants to inherit from a contract which uses types that require it. Use \"pragma abicoder v2;\" for the inheriting contract as well to enable the feature.\ncontract CounterTest is Test {\n^ (Relevant source part starts here and spans across multiple lines).\nlib/forge-std/src/StdInvariant.sol:72:5: Type only supported by ABIEncoderV2\n    function excludeArtifacts() public view returns (string[] memory excludedArtifacts_) {\n    ^ (Relevant source part starts here and spans across multiple lines).\nlib/forge-std/src/StdInvariant.sol:84:5: Type only supported by ABIEncoderV2\n    function targetArtifacts() public view returns (string[] memory targetedArtifacts_) {\n    ^ (Relevant source part starts here and spans across multiple lines).\nlib/forge-std/src/StdInvariant.sol:88:5: Type only supported by ABIEncoderV2\n    function targetArtifactSelectors() public view returns (FuzzSelector[] memory targetedArtifactSelectors_) {\n    ^ (Relevant source part starts here and spans across multiple lines).\nlib/forge-std/src/StdInvariant.sol:96:5: Type only supported by ABIEncoderV2\n    function targetSelectors() public view returns (FuzzSelector[] memory targetedSelectors_) {\n    ^ (Relevant source part starts here and spans across multiple lines).\nlib/forge-std/src/StdInvariant.sol:104:5: Type only supported by ABIEncoderV2\n    function targetInterfaces() public view returns (FuzzInterface[] memory targetedInterfaces_) {\n    ^ (Relevant source part starts here and spans across multiple lines).\n","message":"Contract \"CounterTest\" does not use ABI coder v2 but wants to inherit from a contract which uses types that require it. Use \"pragma abicoder v2;\" for the inheriting contract as well to enable the feature.","secondarySourceLocations":[{"end":2298,"file":"lib/forge-std/src/StdInvariant.sol","message":"Type only supported by ABIEncoderV2","start":2157},{"end":2732,"file":"lib/forge-std/src/StdInvariant.sol","message":"Type only supported by ABIEncoderV2","start":2592},{"end":2916,"file":"lib/forge-std/src/StdInvariant.sol","message":"Type only supported by ABIEncoderV2","start":2738},{"end":3215,"file":"lib/forge-std/src/StdInvariant.sol","message":"Type only supported by ABIEncoderV2","start":3069},{"end":3511,"file":"lib/forge-std/src/StdInvariant.sol","message":"Type only supported by ABIEncoderV2","start":3360}],"severity":"error","sourceLocation":{"end":558,"file":"test/Counter.t.sol","start":157},"type":"TypeError"}],"sources":{}}"#;
        let crate::CompilerOutput { errors, .. } = serde_json::from_str(output).unwrap();
        assert_eq!(errors.len(), 1);
        let s = errors[0].to_string();
        eprintln!("{s}");
        assert!(s.contains("test/Counter.t.sol:7:1"), "\n{s}");
        assert!(s.contains("ABI coder v2"), "\n{s}");
    }

    #[test]
    fn no_source_location() {
        let error = r#"{"component":"general","errorCode":"6553","formattedMessage":"SyntaxError: The msize instruction cannot be used when the Yul optimizer is activated because it can change its semantics. Either disable the Yul optimizer or do not use the instruction.\n\n","message":"The msize instruction cannot be used when the Yul optimizer is activated because it can change its semantics. Either disable the Yul optimizer or do not use the instruction.","severity":"error","sourceLocation":{"end":173,"file":"","start":114},"type":"SyntaxError"}"#;
        let error = serde_json::from_str::<Error>(error).unwrap();
        let s = error.to_string();
        eprintln!("{s}");
        assert!(s.contains("Error (6553)"), "\n{s}");
        assert!(s.contains("The msize instruction cannot be used"), "\n{s}");
    }

    #[test]
    fn no_source_location2() {
        let error = r#"{"component":"general","errorCode":"5667","formattedMessage":"Warning: Unused function parameter. Remove or comment out the variable name to silence this warning.\n\n","message":"Unused function parameter. Remove or comment out the variable name to silence this warning.","severity":"warning","sourceLocation":{"end":104,"file":"","start":95},"type":"Warning"}"#;
        let error = serde_json::from_str::<Error>(error).unwrap();
        let s = error.to_string();
        eprintln!("{s}");
        assert!(s.contains("Warning (5667)"), "\n{s}");
        assert!(s.contains("Unused function parameter. Remove or comment out the variable name to silence this warning."), "\n{s}");
    }

    #[test]
    fn stack_too_deep_multiline() {
        let error = r#"{"sourceLocation":{"file":"test/LibMap.t.sol","start":15084,"end":15113},"type":"YulException","component":"general","severity":"error","errorCode":null,"message":"Yul exception:Cannot swap Variable _23 with Slot RET[fun_assertEq]: too deep in the stack by 1 slots in [ var_136614_mpos RET _23 _21 _23 var_map_136608_slot _34 _34 _29 _33 _33 _39 expr_48 var_bitWidth var_map_136608_slot _26 _29 var_bitWidth TMP[eq, 0] RET[fun_assertEq] ]\nmemoryguard was present.","formattedMessage":"YulException: Cannot swap Variable _23 with Slot RET[fun_assertEq]: too deep in the stack by 1 slots in [ var_136614_mpos RET _23 _21 _23 var_map_136608_slot _34 _34 _29 _33 _33 _39 expr_48 var_bitWidth var_map_136608_slot _26 _29 var_bitWidth TMP[eq, 0] RET[fun_assertEq] ]\nmemoryguard was present.\n   --> test/LibMap.t.sol:461:34:\n    |\n461 |             uint256 end = t.o - (t.o > 0 ? _random() % t.o : 0);\n    |                                  ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^\n\n"}"#;
        let error = serde_json::from_str::<Error>(error).unwrap();
        let s = error.to_string();
        eprintln!("{s}");
        assert_eq!(s.match_indices("Cannot swap Variable _23").count(), 1, "\n{s}");
        assert!(s.contains("-->"), "\n{s}");
    }

    #[test]
    fn stack_too_deep_no_source_location() {
        let error = r#"{"type":"CompilerError","component":"general","severity":"error","errorCode":null,"message":"Compiler error (/solidity/libyul/backends/evm/AsmCodeGen.cpp:63):Stack too deep. Try compiling with `--via-ir` (cli) or the equivalent `viaIR: true` (standard JSON) while enabling the optimizer. Otherwise, try removing local variables. When compiling inline assembly: Variable key_ is 2 slot(s) too deep inside the stack. Stack too deep. Try compiling with `--via-ir` (cli) or the equivalent `viaIR: true` (standard JSON) while enabling the optimizer. Otherwise, try removing local variables.","formattedMessage":"CompilerError: Stack too deep. Try compiling with `--via-ir` (cli) or the equivalent `viaIR: true` (standard JSON) while enabling the optimizer. Otherwise, try removing local variables. When compiling inline assembly: Variable key_ is 2 slot(s) too deep inside the stack. Stack too deep. Try compiling with `--via-ir` (cli) or the equivalent `viaIR: true` (standard JSON) while enabling the optimizer. Otherwise, try removing local variables.\n\n"}"#;
        let error = serde_json::from_str::<Error>(error).unwrap();
        let s = error.to_string();
        eprintln!("{s}");
        assert_eq!(s.match_indices("too deep inside the stack.").count(), 1, "\n{s}");
    }
}
