#![warn(clippy::pedantic)]
#![allow(clippy::uninlined_format_args)]
use core::fmt::{Debug, Display, Formatter};
use core::ops::RangeInclusive;

const UNIT_SEP: &str = "bytes=";
/// Function that parses the content of a range header.
///
/// Follows the [spec here](https://developer.mozilla.org/en-US/docs/Web/HTTP/Headers/Range)
///
/// And [here](https://www.ietf.org/rfc/rfc2616.txt)
///
/// Will only accept bytes ranges, will update when [this spec](https://www.iana.org/assignments/http-parameters/http-parameters.xhtml) changes to allow other units.
///
/// Parses ranges strictly, as in the examples contained in the above specifications.
///
/// Ranges such as `bytes=0-15, 16-20, abc` will be rejected immediately.
///
/// It preserves the range ordering, the specification leaves it open to the server to determine whether
/// ranges that are out of order are correct or not, ie `bytes=20-30, 0-15`
///
/// # Example no trailing or leading whitespaces
/// ```
/// // Ok
/// assert!(http_range_header::parse_range_header("bytes=0-15").is_ok());
/// // Not allowed
/// assert!(http_range_header::parse_range_header("bytes=0-15 ").is_err());
/// // Also not allowed
/// assert!(http_range_header::parse_range_header("bytes= 0-15").is_err());
/// ```
///
/// # Example No leading whitespaces except in the case of separating multipart ranges
/// ```
/// // Ok, multipart with a leading whitespace after comma
/// assert!(http_range_header::parse_range_header("bytes=0-15, 20-30").is_ok());
/// // Ok multipart without leading whitespace after comma
/// assert!(http_range_header::parse_range_header("bytes=0-15,20-30").is_ok());
/// ```
///
/// # Example No negative values, no leading zeroes, no plus-sign
/// ```
/// // No negatives
/// assert!(http_range_header::parse_range_header("bytes=-12-15").is_err());
/// // No leading zeroes
/// assert!(http_range_header::parse_range_header("bytes=00-15").is_err());
/// // No plus sign
/// assert!(http_range_header::parse_range_header("bytes=+0-15").is_err());
/// ```
///
/// Makes two passes and parses ranges strictly. On the first pass, if any range is malformed returns an `Err`.
///
/// On the second pass if the ranges doesn't make sense (reversed range, range out of bounds, etc.) returns an `Err`.
/// # Example with a standard valid range
///
/// ```
/// let input = "bytes=0-15";
/// let file_size_bytes = 512;
/// let parsed_ranges = http_range_header::parse_range_header(input);
///
/// match parsed_ranges {
///     Ok(ranges) => {
///         match ranges.validate(file_size_bytes) {
///             Ok(valid_ranges) => {
///                 for range in valid_ranges {
///                     // do something with ranges
///                     assert_eq!(0..=15, range)
///                 }
///             }
///             Err(_err) => {
///                 // Do something when ranges doesn't make sense
///                 panic!("Weird range!")
///             }
///         }
///     }
///     Err(_err) => {
///         // Do something with malformed ranges
///         panic!("Malformed range!")
///     }
/// }
/// ```
///
/// The parser makes two passes, one without a known file-size, ensuring all ranges are syntactically correct.
/// The returned struct will through its `validate` method accept a file-size and figure out whether or not the
/// syntactically correct ranges actually makes sense in context
///
/// The range `bytes=0-20` on a file with 15 bytes will be accepted in the first pass as the content size is unknown.
/// On the second pass (`validate`) it will be truncated to `file_size - 1` as per [the spec](https://httpwg.org/specs/rfc9110.html#rfc.section.14.1.2).
/// # Example range truncates in `validate` because it exceedes
/// ```
/// let input = "bytes=0-20";
/// let file_size_bytes = 15;
/// let parsed_ranges = http_range_header::parse_range_header(input)
///     // Is syntactically correct
///     .unwrap();
/// let validated = parsed_ranges.validate(file_size_bytes).unwrap();
/// assert_eq!(vec![0..=14], validated);
/// ```
///
/// Range reversal and overlap is also checked in the second pass, the range `bytes=0-20, 5-10`
/// will become two syntactically correct ranges, but `validate` will return ann `Err`.
///
/// This is an opinionated implementation, [the spec](https://datatracker.ietf.org/doc/html/rfc7233)
/// allows a server to determine its implementation of overlapping ranges, this api currently does not allow it.
///
/// # Example multipart-range fails `validate` because of an overlap
/// ```
/// let input = "bytes=0-15, 10-20, 30-50";
/// let file_size_bytes = 512;
/// let parsed_ranges = http_range_header::parse_range_header(input)
///     // Is syntactically correct
///     .unwrap();
/// let validated = parsed_ranges.validate(file_size_bytes);
/// // Some ranges overlap, all valid ranges get truncated to 1 Err
/// assert!(validated.is_err());
/// ```
/// # Errors
/// Will return an error if the `range_header_value` cannot be strictly parsed into a range
/// per the http spec.
pub fn parse_range_header(
    range_header_value: &str,
) -> Result<ParsedRanges, RangeUnsatisfiableError> {
    const COMMA: char = ',';
    if let Some((prefix, indicated_range)) = range_header_value.split_once(UNIT_SEP) {
        if indicated_range.starts_with(char::is_whitespace) {
            return Err(RangeUnsatisfiableError::StartsWithWhitespace);
        }
        if !prefix.is_empty() {
            return Err(RangeUnsatisfiableError::DoesNotStartWithToken);
        }
        let mut last_err = None;
        let ranges = indicated_range
            .split(COMMA)
            .filter_map(|range| {
                if let Some(trimmed) = trim(range) {
                    match parse_inner(trimmed) {
                        Ok(parsed) => Some(parsed),
                        Err(e) => {
                            last_err = Some(e);
                            None
                        }
                    }
                } else {
                    last_err = Some(RangeUnsatisfiableError::IllegalWhitespace);
                    None
                }
            })
            .collect::<Vec<SyntacticallyCorrectRange>>();
        if let Some(last_err) = last_err {
            return Err(last_err);
        }
        if ranges.is_empty() {
            // Some other error should have been caught before we end up here
            Err(RangeUnsatisfiableError::Empty)
        } else {
            Ok(ParsedRanges::new(ranges))
        }
    } else {
        Err(RangeUnsatisfiableError::DoesNotStartWithToken)
    }
}

fn trim(s: &str) -> Option<&str> {
    if s.ends_with(char::is_whitespace) || s.match_indices(char::is_whitespace).count() > 1 {
        None
    } else {
        Some(s.trim())
    }
}

#[inline]
fn parse_inner(range: &str) -> Result<SyntacticallyCorrectRange, RangeUnsatisfiableError> {
    if let Some((start, end)) = range.split_once('-') {
        if start.is_empty() {
            if let Some(end) = strict_parse_u64(end) {
                if end == 0 {
                    return Err(RangeUnsatisfiableError::ZeroSuffix);
                }
                return Ok(SyntacticallyCorrectRange::new(
                    StartPosition::FromLast(end),
                    EndPosition::LastByte,
                ));
            }
            return Err(RangeUnsatisfiableError::BadEndOfRange);
        }
        if let Some(start) = strict_parse_u64(start) {
            if end.is_empty() {
                return Ok(SyntacticallyCorrectRange::new(
                    StartPosition::Index(start),
                    EndPosition::LastByte,
                ));
            }
            if let Some(end) = strict_parse_u64(end) {
                return Ok(SyntacticallyCorrectRange::new(
                    StartPosition::Index(start),
                    EndPosition::Index(end),
                ));
            }
            return Err(RangeUnsatisfiableError::BadEndOfRange);
        }
        return Err(RangeUnsatisfiableError::BadStartOfRange);
    }
    Err(RangeUnsatisfiableError::UnexpectedNumberOfDashes)
}

fn strict_parse_u64(s: &str) -> Option<u64> {
    if !s.starts_with('+') && (s.len() == 1 || !s.starts_with('0')) {
        return s.parse::<u64>().ok();
    }
    None
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ParsedRanges {
    pub ranges: Vec<SyntacticallyCorrectRange>,
}

impl ParsedRanges {
    fn new(ranges: Vec<SyntacticallyCorrectRange>) -> Self {
        ParsedRanges { ranges }
    }

    /// Validates a parsed range for a given file-size in bytes.
    /// # Errors
    /// If the range is invalid for the the file-size.
    pub fn validate(
        &self,
        file_size_bytes: u64,
    ) -> Result<Vec<RangeInclusive<u64>>, RangeUnsatisfiableError> {
        let len = self.ranges.len();
        let mut validated = Vec::with_capacity(len);
        for parsed in &self.ranges {
            let start = match parsed.start {
                StartPosition::Index(i) => i,
                StartPosition::FromLast(i) => {
                    if i > file_size_bytes {
                        return Err(RangeUnsatisfiableError::FileSuffixOutOfBounds);
                    }
                    file_size_bytes.saturating_sub(i)
                }
            };
            let end = match parsed.end {
                EndPosition::Index(i) => core::cmp::min(i, file_size_bytes.saturating_sub(1)),
                EndPosition::LastByte => file_size_bytes.saturating_sub(1),
            };

            let valid = RangeInclusive::new(start, end);
            validated.push(valid);
        }
        // False positive
        #[allow(clippy::match_same_arms)]
        match validate_ranges(validated.as_slice()) {
            RangeValidationResult::Valid => Ok(validated),
            RangeValidationResult::Overlapping => Err(RangeUnsatisfiableError::OverlappingRanges),
            RangeValidationResult::Reversed => Err(RangeUnsatisfiableError::RangeReversed),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RangeUnsatisfiableError {
    OverlappingRanges,
    RangeReversed,
    FileSuffixOutOfBounds,
    IllegalWhitespace,
    StartsWithWhitespace,
    DoesNotStartWithToken,
    ZeroSuffix,
    BadStartOfRange,
    BadEndOfRange,
    UnexpectedNumberOfDashes,
    Empty,
}

impl Display for RangeUnsatisfiableError {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        match self {
            RangeUnsatisfiableError::OverlappingRanges => {
                f.write_str("RangeUnsatisfiable: Ranges overlap")
            }
            RangeUnsatisfiableError::RangeReversed => {
                f.write_str("RangeUnsatisfiable: Reversed range")
            }
            RangeUnsatisfiableError::FileSuffixOutOfBounds => f.write_str(
                "RangeUnsatisfiable: File suffix out of bounds (larger than file bytes)",
            ),
            RangeUnsatisfiableError::IllegalWhitespace => {
                f.write_str("RangeUnsatisfiable: Illegal whitespaces in range")
            }
            RangeUnsatisfiableError::StartsWithWhitespace => {
                f.write_str("RangeUnsatisfiable: Range starts with whitespace")
            }
            RangeUnsatisfiableError::DoesNotStartWithToken => f.write_fmt(format_args!(
                "RangeUnsatisfiable: Range does not start with token '{UNIT_SEP}'"
            )),
            RangeUnsatisfiableError::ZeroSuffix => {
                f.write_str("RangeUnsatisfiable: Range ends at 0")
            }
            RangeUnsatisfiableError::BadStartOfRange => {
                f.write_str("RangeUnsatisfiable: Unparseable start of range")
            }
            RangeUnsatisfiableError::BadEndOfRange => {
                f.write_str("RangeUnsatisfiable: Unparseable end of range")
            }
            RangeUnsatisfiableError::UnexpectedNumberOfDashes => {
                f.write_str("RangeUnsatisfiable: Unexpected number of dashes")
            }
            RangeUnsatisfiableError::Empty => f.write_str(
                "RangeUnsatisfiable: Failed to parse range fallback error, please file an issue",
            ),
        }
    }
}

impl std::error::Error for RangeUnsatisfiableError {}

enum RangeValidationResult {
    Valid,
    Overlapping,
    Reversed,
}

fn validate_ranges(ranges: &[RangeInclusive<u64>]) -> RangeValidationResult {
    let mut bounds = Vec::new();
    for range in ranges {
        let start = range.start();
        let end = range.end();
        if start > end {
            return RangeValidationResult::Reversed;
        } else if ranges.len() == 1 {
            return RangeValidationResult::Valid;
        }
        bounds.push((range.start(), range.end()));
    }
    for i in 0..bounds.len() {
        for j in i + 1..bounds.len() {
            if bounds[i].0 <= bounds[j].1 && bounds[j].0 <= bounds[i].1 {
                return RangeValidationResult::Overlapping;
            }
        }
    }
    RangeValidationResult::Valid
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct SyntacticallyCorrectRange {
    pub start: StartPosition,
    pub end: EndPosition,
}

impl SyntacticallyCorrectRange {
    fn new(start: StartPosition, end: EndPosition) -> Self {
        SyntacticallyCorrectRange { start, end }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StartPosition {
    Index(u64),
    FromLast(u64),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum EndPosition {
    Index(u64),
    LastByte,
}

#[cfg(test)]
mod tests {
    use crate::{
        parse_range_header, EndPosition, ParsedRanges, RangeUnsatisfiableError, StartPosition,
        SyntacticallyCorrectRange,
    };
    use core::ops::RangeInclusive;

    const TEST_FILE_LENGTH: u64 = 10_000;
    /// Testing standard range compliance against <https://datatracker.ietf.org/doc/html/rfc7233>
    #[test]
    fn rfc_7233_standard_test1() {
        let input = "bytes=0-499";
        let expect =
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(499));
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(0, 499);
        let actual = actual.validate(TEST_FILE_LENGTH).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    #[test]
    fn rfc_7233_standard_test2() {
        let input = "bytes=500-999";
        let expect =
            SyntacticallyCorrectRange::new(StartPosition::Index(500), EndPosition::Index(999));
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(500, 999);
        let actual = actual.validate(TEST_FILE_LENGTH).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    /// Testing suffix compliance against <https://datatracker.ietf.org/doc/html/rfc7233>
    #[test]
    fn rfc_7233_suffixed_test() {
        let input = "bytes=-500";
        let expect =
            SyntacticallyCorrectRange::new(StartPosition::FromLast(500), EndPosition::LastByte);
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(9500, 9999);
        let actual = actual.validate(10_000).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    /// Testing open range compliance against <https://datatracker.ietf.org/doc/html/rfc7233>
    #[test]
    fn rfc_7233_open_range_test() {
        let input = "bytes=9500-";
        let expect =
            SyntacticallyCorrectRange::new(StartPosition::Index(9500), EndPosition::LastByte);
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(9500, 9999);
        let actual = actual.validate(10_000).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    /// Testing first and last bytes compliance against <https://datatracker.ietf.org/doc/html/rfc7233>
    #[test]
    fn rfc_7233_first_and_last() {
        let input = "bytes=0-0, -1";
        let expect = vec![
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(0)),
            SyntacticallyCorrectRange::new(StartPosition::FromLast(1), EndPosition::LastByte),
        ];
        let actual = parse_range_header(input).unwrap();
        assert_eq!(expect, actual.ranges);
        let expect = vec![0..=0, 9999..=9999];
        let actual = actual.validate(10_000).unwrap();
        assert_eq!(expect, actual);
    }

    #[test]
    fn parse_standard_range() {
        let input = "bytes=0-1023";
        let expect =
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(1023));
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(0, 1023);
        let actual = actual.validate(TEST_FILE_LENGTH).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    #[test]
    fn parse_open_ended_range() {
        let input = "bytes=0-";
        let expect = SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::LastByte);
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(0, TEST_FILE_LENGTH - 1);
        let actual = actual.validate(TEST_FILE_LENGTH).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    #[test]
    fn parse_suffix_range_edge() {
        let input = &format!("bytes=-{}", TEST_FILE_LENGTH);
        let expect = SyntacticallyCorrectRange::new(
            StartPosition::FromLast(TEST_FILE_LENGTH),
            EndPosition::LastByte,
        );
        let actual = parse_range_header(input).unwrap();
        assert_eq!(single_range(expect), actual);
        let expect = RangeInclusive::new(0, TEST_FILE_LENGTH - 1);
        let actual = actual.validate(TEST_FILE_LENGTH).unwrap()[0].clone();
        assert_eq!(expect, actual);
    }

    #[test]
    fn parse_empty_as_invalid() {
        let input = "";
        let parsed = parse_range_header(input);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::DoesNotStartWithToken));
    }

    #[test]
    fn parse_empty_range_as_invalid() {
        let input = "bytes=";
        let parsed = parse_range_header(input);
        // 0 is unexpected
        assert_eq!(
            parsed,
            Err(RangeUnsatisfiableError::UnexpectedNumberOfDashes)
        );
    }

    #[test]
    fn parse_range_starting_with_whitespace_as_invalid() {
        let input = "bytes= 0-15";
        let parsed = parse_range_header(input);
        // 0 is unexpected
        assert_eq!(parsed, Err(RangeUnsatisfiableError::StartsWithWhitespace));
    }

    #[test]
    fn parse_range_token_starting_with_whitespace_as_invalid() {
        let input = " bytes=0-15";
        let parsed = parse_range_header(input);
        // 0 is unexpected
        assert_eq!(parsed, Err(RangeUnsatisfiableError::DoesNotStartWithToken));
    }

    #[test]
    fn parse_range_strict_parse_numerical() {
        let input = "bytes=+0-15";
        let parsed = parse_range_header(input);
        // 0 is unexpected
        assert_eq!(parsed, Err(RangeUnsatisfiableError::BadStartOfRange));
    }

    #[test]
    fn parse_bad_unit_as_invalid() {
        let input = "abcde=0-10";
        let parsed = parse_range_header(input);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::DoesNotStartWithToken));
    }

    #[test]
    fn parse_missing_equals_as_malformed() {
        let input = "bytes0-10";
        let parsed = parse_range_header(input);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::DoesNotStartWithToken));
    }

    #[test]
    fn parse_negative_bad_characters_in_range_as_malformed() {
        let input = "bytes=1-10a";
        let parsed = parse_range_header(input);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::BadEndOfRange));
    }

    #[test]
    fn parse_negative_numbers_as_malformed() {
        let input = "bytes=-1-10";
        let parsed = parse_range_header(input);
        // Becomes bad eor, since -1 signals suffixed range
        assert_eq!(parsed, Err(RangeUnsatisfiableError::BadEndOfRange));
    }

    #[test]
    fn parse_bad_characters_in_start_of_range() {
        let input = "bytes=a1-10";
        let parsed = parse_range_header(input);
        // Becomes bad eor, since -1 signals suffixed range
        assert_eq!(parsed, Err(RangeUnsatisfiableError::BadStartOfRange));
    }

    #[test]
    fn parse_out_of_bounds_overrun_as_content_length() {
        let input = &format!("bytes=0-{}", TEST_FILE_LENGTH);
        let expect = vec![RangeInclusive::new(0, TEST_FILE_LENGTH - 1)];
        let actual = parse_range_header(input)
            .unwrap()
            .validate(TEST_FILE_LENGTH)
            .unwrap();
        assert_eq!(expect, actual);
    }

    #[test]
    fn parse_out_of_bounds_suffix_overrun_as_unsatisfiable() {
        let input = &format!("bytes=-{}", TEST_FILE_LENGTH + 1);
        let parsed = parse_range_header(input)
            .unwrap()
            .validate(TEST_FILE_LENGTH);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::FileSuffixOutOfBounds));
    }

    #[test]
    fn parse_zero_length_suffix_as_unsatisfiable() {
        let input = "bytes=-0";
        let parsed = parse_range_header(input);
        assert_eq!(parsed, Err(RangeUnsatisfiableError::ZeroSuffix));
    }

    #[test]
    fn parse_single_reversed_as_invalid() {
        let input = "bytes=15-0";
        let parsed = parse_range_header(input).unwrap();
        assert_eq!(
            parsed.validate(TEST_FILE_LENGTH),
            Err(RangeUnsatisfiableError::RangeReversed)
        );
    }

    #[test]
    fn parse_zero_range_as_invalid() {
        let input = "bytes=15-";
        let parsed = parse_range_header(input).unwrap();
        assert_eq!(
            parsed.validate(0),
            Err(RangeUnsatisfiableError::RangeReversed)
        );
    }

    #[test]
    fn parse_zero_range_last_byte_valid_if_file_size_0() {
        let input = "bytes=0-";
        let expect = SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::LastByte);
        let actual = parse_range_header(input).unwrap().ranges[0];
        assert_eq!(actual, expect);
    }

    #[test]
    fn parse_zero_range_closed_valid_if_file_size_0() {
        let input = "bytes=0-0";
        let expect = SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(0));
        let actual = parse_range_header(input).unwrap().ranges[0];
        assert_eq!(actual, expect);
    }

    #[test]
    fn parse_multi_range() {
        let input = "bytes=0-1023, 2015-3000, 4000-4500, 8000-9999";
        let expected_ranges = vec![
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(1023)),
            SyntacticallyCorrectRange::new(StartPosition::Index(2015), EndPosition::Index(3000)),
            SyntacticallyCorrectRange::new(StartPosition::Index(4000), EndPosition::Index(4500)),
            SyntacticallyCorrectRange::new(StartPosition::Index(8000), EndPosition::Index(9999)),
        ];
        let parsed = parse_range_header(input).unwrap();
        assert_eq!(expected_ranges, parsed.ranges);
        let validated = parsed.validate(TEST_FILE_LENGTH).unwrap();
        assert_eq!(
            vec![0..=1023, 2015..=3000, 4000..=4500, 8000..=9999],
            validated
        );
    }

    #[test]
    fn parse_multi_range_with_open() {
        let input = "bytes=0-1023, 1024-";
        let expected_ranges = vec![
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(1023)),
            SyntacticallyCorrectRange::new(StartPosition::Index(1024), EndPosition::LastByte),
        ];
        let parsed = parse_range_header(input).unwrap();
        assert_eq!(expected_ranges, parsed.ranges);
        let validated = parsed.validate(TEST_FILE_LENGTH).unwrap();
        assert_eq!(vec![0..=1023, 1024..=9999], validated);
    }

    #[test]
    fn parse_multi_range_with_suffix() {
        let input = "bytes=0-1023, -1000";
        let expected_ranges = vec![
            SyntacticallyCorrectRange::new(StartPosition::Index(0), EndPosition::Index(1023)),
            SyntacticallyCorrectRange::new(StartPosition::FromLast(1000), EndPosition::LastByte),
        ];
        let parsed = parse_range_header(input).unwrap();
        assert_eq!(expected_ranges, parsed.ranges);
        assert_eq!(expected_ranges, parsed.ranges);
        let validated = parsed.validate(TEST_FILE_LENGTH).unwrap();
        assert_eq!(vec![0..=1023, 9000..=9999], validated);
    }

    #[test]
    fn parse_overlapping_multi_range_as_unsatisfiable_standard() {
        let input = "bytes=0-1023, 500-800";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
        let input = "bytes=0-0, 0-15";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
        let input = "bytes=0-20, 20-35";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
    }

    #[test]
    fn parse_overlapping_multi_range_as_unsatisfiable_open() {
        let input = "bytes=0-, 5000-6000";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
    }

    #[test]
    fn parse_overlapping_multi_range_as_unsatisfiable_suffixed() {
        let input = "bytes=8000-9000, -1001";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
        let input = "bytes=8000-9000, -1000";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
        // This doesn't overlap
        let input = "bytes=8000-9000, -999";
        parse_range_header(input)
            .unwrap()
            .validate(TEST_FILE_LENGTH)
            .unwrap();
    }

    #[test]
    fn parse_overlapping_multi_range_as_unsatisfiable_suffixed_open() {
        let input = "bytes=0-, -1";
        assert_validation_err(input, RangeUnsatisfiableError::OverlappingRanges);
    }

    #[test]
    fn parse_multi_range_with_a_reversed_as_invalid() {
        let input = "bytes=0-15, 30-20";
        assert_validation_err(input, RangeUnsatisfiableError::RangeReversed);
    }

    fn assert_validation_err(input: &str, err: RangeUnsatisfiableError) {
        let parsed = parse_range_header(input)
            .unwrap()
            .validate(TEST_FILE_LENGTH);
        assert_eq!(Err(err), parsed);
    }

    #[test]
    fn parse_multi_range_rejects_invalid() {
        let input = "bytes=0-15, 25, 9, ";
        let parsed = parse_range_header(input);
        assert!(parsed.is_err());
    }

    #[quickcheck_macros::quickcheck]
    #[allow(clippy::needless_pass_by_value)]
    fn always_errs_on_random_input(input: String) -> quickcheck::TestResult {
        // Basic regex matching most valid range headers
        let acceptable = regex::Regex::new(
            "^bytes=((\\d+-\\d+,\\s?)|(\\d+-,\\s?)|(-\\d+,\\s?))*((\\d+-\\d+)|(\\d+-)|(-\\d+))+$",
        )
        .unwrap();
        if acceptable.is_match(&input) {
            quickcheck::TestResult::discard()
        } else if let Ok(passed_first_pass) = parse_range_header(&input) {
            quickcheck::TestResult::from_bool(passed_first_pass.validate(u64::MAX).is_err())
        } else {
            quickcheck::TestResult::passed()
        }
    }

    fn single_range(syntactically_correct: SyntacticallyCorrectRange) -> ParsedRanges {
        ParsedRanges::new(vec![syntactically_correct])
    }
}
