use std::{fmt, ops, time::Duration};

use crate::util;

/// [Picosecond](https://en.wikipedia.org/wiki/Picosecond)-precise [`Duration`].
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub(crate) struct FineDuration {
    pub picos: u128,
}

impl From<Duration> for FineDuration {
    #[inline]
    fn from(duration: Duration) -> Self {
        Self {
            picos: duration
                .as_nanos()
                .checked_mul(1_000)
                .unwrap_or_else(|| panic!("{duration:?} is too large to fit in `FineDuration`")),
        }
    }
}

impl fmt::Display for FineDuration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let sig_figs = f.precision().unwrap_or(4);

        let picos = self.picos;
        let mut scale = TimeScale::from_picos(picos);

        // Prefer formatting picoseconds as nanoseconds if we can. This makes
        // picoseconds easier to read because they are almost always alongside
        // nanosecond-scale values.
        if scale == TimeScale::PicoSec && sig_figs > 3 {
            scale = TimeScale::NanoSec;
        }

        let multiple: u128 = {
            let sig_figs = u32::try_from(sig_figs).unwrap_or(u32::MAX);
            10_u128.saturating_pow(sig_figs)
        };

        // TODO: Format without heap allocation.
        let mut str: String = match picos::DAY.checked_mul(multiple) {
            Some(int_day) if picos >= int_day => {
                // Format using integer representation to not lose precision.
                (picos / picos::DAY).to_string()
            }
            _ => {
                // Format using floating point representation.

                // Multiply to allow `sig_figs` digits of fractional precision.
                let val = (((picos * multiple) / scale.picos()) as f64) / multiple as f64;

                util::fmt::format_f64(val, sig_figs)
            }
        };

        str.push(' ');
        str.push_str(scale.suffix());

        // Fill up to specified width.
        if let Some(fill_len) = f.width().and_then(|width| width.checked_sub(str.len())) {
            match f.align() {
                None | Some(fmt::Alignment::Left) => {
                    str.extend(std::iter::repeat(f.fill()).take(fill_len));
                }
                _ => return Err(fmt::Error),
            }
        }

        f.write_str(&str)
    }
}

impl fmt::Debug for FineDuration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl ops::Add for FineDuration {
    type Output = Self;

    #[inline]
    fn add(self, other: Self) -> Self {
        Self { picos: self.picos + other.picos }
    }
}

impl ops::AddAssign for FineDuration {
    #[inline]
    fn add_assign(&mut self, other: Self) {
        self.picos += other.picos
    }
}

impl<I: Into<u128>> ops::Div<I> for FineDuration {
    type Output = Self;

    #[inline]
    fn div(self, count: I) -> Self {
        Self { picos: self.picos / count.into() }
    }
}

impl FineDuration {
    pub const ZERO: Self = Self { picos: 0 };

    pub const MAX: Self = Self { picos: u128::MAX };

    #[inline]
    pub fn is_zero(&self) -> bool {
        self.picos == 0
    }

    /// Round up to `other` if `self` is zero.
    #[inline]
    pub fn clamp_to(self, other: Self) -> Self {
        if self.is_zero() {
            other
        } else {
            self
        }
    }

    /// Returns the smaller non-zero value.
    #[inline]
    pub fn clamp_to_min(self, other: Self) -> Self {
        if self.is_zero() {
            other
        } else if other.is_zero() {
            self
        } else {
            self.min(other)
        }
    }
}

mod picos {
    pub const NANOS: u128 = 1_000;
    pub const MICROS: u128 = 1_000 * NANOS;
    pub const MILLIS: u128 = 1_000 * MICROS;
    pub const SEC: u128 = 1_000 * MILLIS;
    pub const MIN: u128 = 60 * SEC;
    pub const HOUR: u128 = 60 * MIN;
    pub const DAY: u128 = 24 * HOUR;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum TimeScale {
    PicoSec,
    NanoSec,
    MicroSec,
    MilliSec,
    Sec,
    Min,
    Hour,
    Day,
}

impl TimeScale {
    #[cfg(test)]
    const ALL: &'static [Self] = &[
        Self::PicoSec,
        Self::NanoSec,
        Self::MicroSec,
        Self::MilliSec,
        Self::Sec,
        Self::Min,
        Self::Hour,
        Self::Day,
    ];

    /// Determines the scale of time for representing a number of picoseconds.
    fn from_picos(picos: u128) -> Self {
        use picos::*;

        if picos < NANOS {
            Self::PicoSec
        } else if picos < MICROS {
            Self::NanoSec
        } else if picos < MILLIS {
            Self::MicroSec
        } else if picos < SEC {
            Self::MilliSec
        } else if picos < MIN {
            Self::Sec
        } else if picos < HOUR {
            Self::Min
        } else if picos < DAY {
            Self::Hour
        } else {
            Self::Day
        }
    }

    /// Returns the number of picoseconds needed to reach this scale.
    fn picos(self) -> u128 {
        use picos::*;

        match self {
            Self::PicoSec => 1,
            Self::NanoSec => NANOS,
            Self::MicroSec => MICROS,
            Self::MilliSec => MILLIS,
            Self::Sec => SEC,
            Self::Min => MIN,
            Self::Hour => HOUR,
            Self::Day => DAY,
        }
    }

    /// Returns the unit suffix.
    fn suffix(self) -> &'static str {
        match self {
            Self::PicoSec => "ps",
            Self::NanoSec => "ns",
            Self::MicroSec => "µs",
            Self::MilliSec => "ms",
            Self::Sec => "s",
            Self::Min => "m",
            Self::Hour => "h",
            Self::Day => "d",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_to() {
        #[track_caller]
        fn test(a: u128, b: u128, expected: u128) {
            assert_eq!(
                FineDuration { picos: a }.clamp_to(FineDuration { picos: b }),
                FineDuration { picos: expected }
            );
        }

        test(0, 0, 0);
        test(0, 1, 1);
        test(0, 2, 2);
        test(0, 3, 3);

        test(1, 0, 1);
        test(1, 1, 1);
        test(1, 2, 1);
        test(1, 3, 1);

        test(2, 0, 2);
        test(2, 1, 2);
        test(2, 2, 2);
        test(2, 3, 2);

        test(3, 0, 3);
        test(3, 1, 3);
        test(3, 2, 3);
        test(3, 3, 3);
    }

    #[test]
    fn clamp_to_min() {
        #[track_caller]
        fn test(a: u128, b: u128, expected: u128) {
            assert_eq!(
                FineDuration { picos: a }.clamp_to_min(FineDuration { picos: b }),
                FineDuration { picos: expected }
            );
        }

        test(0, 0, 0);
        test(0, 1, 1);
        test(0, 2, 2);
        test(0, 3, 3);

        test(1, 0, 1);
        test(1, 1, 1);
        test(1, 2, 1);
        test(1, 3, 1);

        test(2, 0, 2);
        test(2, 1, 1);
        test(2, 2, 2);
        test(2, 3, 2);

        test(3, 0, 3);
        test(3, 1, 1);
        test(3, 2, 2);
        test(3, 3, 3);
    }

    #[allow(clippy::zero_prefixed_literal)]
    mod fmt {
        use super::*;

        #[track_caller]
        fn test(picos: u128, expected: &str) {
            let duration = FineDuration { picos };
            assert_eq!(duration.to_string(), expected);
            assert_eq!(format!("{duration:.4}"), expected);
            assert_eq!(format!("{duration:<0}"), expected);
        }

        macro_rules! assert_fmt_eq {
            ($input:literal, $expected:literal) => {
                assert_eq!(format!($input), format!($expected));
            };
        }

        #[test]
        fn precision() {
            for &scale in TimeScale::ALL {
                let base_duration = FineDuration { picos: scale.picos() };
                let incr_duration = FineDuration { picos: scale.picos() + 1 };

                if scale == TimeScale::PicoSec {
                    assert_eq!(format!("{base_duration:.0}"), "1 ps");
                    assert_eq!(format!("{incr_duration:.0}"), "2 ps");
                } else {
                    let base_string = base_duration.to_string();
                    assert_eq!(format!("{base_duration:.0}"), base_string);
                    assert_eq!(format!("{incr_duration:.0}"), base_string);
                }
            }
        }

        #[test]
        fn fill() {
            for &scale in TimeScale::ALL {
                // Picoseconds are formatted as nanoseconds by default.
                if scale == TimeScale::PicoSec {
                    continue;
                }

                let duration = FineDuration { picos: scale.picos() };
                let suffix = scale.suffix();
                let pad = " ".repeat(8 - suffix.len());

                assert_fmt_eq!("{duration:<2}", "1 {suffix}");
                assert_fmt_eq!("{duration:<10}", "1 {suffix}{pad}");
            }
        }

        #[test]
        fn pico_sec() {
            test(000, "0 ns");

            test(001, "0.001 ns");
            test(010, "0.01 ns");
            test(100, "0.1 ns");

            test(102, "0.102 ns");
            test(120, "0.12 ns");
            test(123, "0.123 ns");
            test(012, "0.012 ns");
        }

        #[test]
        fn nano_sec() {
            test(001_000, "1 ns");
            test(010_000, "10 ns");
            test(100_000, "100 ns");

            test(100_002, "100 ns");
            test(100_020, "100 ns");
            test(100_200, "100.2 ns");
            test(102_000, "102 ns");
            test(120_000, "120 ns");

            test(001_002, "1.002 ns");
            test(001_023, "1.023 ns");
            test(001_234, "1.234 ns");
            test(001_230, "1.23 ns");
            test(001_200, "1.2 ns");
        }

        #[test]
        fn micro_sec() {
            test(001_000_000, "1 µs");
            test(010_000_000, "10 µs");
            test(100_000_000, "100 µs");

            test(100_000_002, "100 µs");
            test(100_000_020, "100 µs");
            test(100_000_200, "100 µs");
            test(100_002_000, "100 µs");
            test(100_020_000, "100 µs");
            test(100_200_000, "100.2 µs");
            test(102_000_000, "102 µs");

            test(120_000_000, "120 µs");
            test(012_000_000, "12 µs");
            test(001_200_000, "1.2 µs");

            test(001_020_000, "1.02 µs");
            test(001_002_000, "1.002 µs");
            test(001_000_200, "1 µs");
            test(001_000_020, "1 µs");
            test(001_000_002, "1 µs");

            test(001_230_000, "1.23 µs");
            test(001_234_000, "1.234 µs");
            test(001_234_500, "1.234 µs");
            test(001_234_560, "1.234 µs");
            test(001_234_567, "1.234 µs");
        }

        #[test]
        fn milli_sec() {
            test(001_000_000_000, "1 ms");
            test(010_000_000_000, "10 ms");
            test(100_000_000_000, "100 ms");
        }

        #[test]
        fn sec() {
            test(picos::SEC, "1 s");
            test(picos::SEC * 10, "10 s");
            test(picos::SEC * 59, "59 s");

            test(picos::MILLIS * 59_999, "59.99 s");
        }

        #[test]
        fn min() {
            test(picos::MIN, "1 m");
            test(picos::MIN * 10, "10 m");
            test(picos::MIN * 59, "59 m");

            test(picos::MILLIS * 3_599_000, "59.98 m");
            test(picos::MILLIS * 3_599_999, "59.99 m");
            test(picos::HOUR - 1, "59.99 m");
        }

        #[test]
        fn hour() {
            test(picos::HOUR, "1 h");
            test(picos::HOUR * 10, "10 h");
            test(picos::HOUR * 23, "23 h");

            test(picos::MILLIS * 86_300_000, "23.97 h");
            test(picos::MILLIS * 86_399_999, "23.99 h");
            test(picos::DAY - 1, "23.99 h");
        }

        #[test]
        fn day() {
            test(picos::DAY, "1 d");

            test(picos::DAY + picos::DAY / 10, "1.1 d");
            test(picos::DAY + picos::DAY / 100, "1.01 d");
            test(picos::DAY + picos::DAY / 1000, "1.001 d");

            test(picos::DAY * 000010, "10 d");
            test(picos::DAY * 000100, "100 d");
            test(picos::DAY * 001000, "1000 d");
            test(picos::DAY * 010000, "10000 d");
            test(picos::DAY * 100000, "100000 d");

            test(u128::MAX / 1000, "3938453320844195178 d");
            test(u128::MAX, "3938453320844195178974 d");
        }
    }
}
