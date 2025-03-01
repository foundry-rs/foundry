use std::fmt;

use crate::counter::{AnyCounter, BytesFormat, KnownCounterKind};

/// Formats an `f64` to the given number of significant figures.
pub(crate) fn format_f64(val: f64, sig_figs: usize) -> String {
    let mut str = val.to_string();

    if let Some(dot_index) = str.find('.') {
        let fract_digits = sig_figs.saturating_sub(dot_index);

        if fract_digits == 0 {
            str.truncate(dot_index);
        } else {
            let fract_start = dot_index + 1;
            let fract_end = fract_start + fract_digits;
            let fract_range = fract_start..fract_end;

            if let Some(fract_str) = str.get(fract_range) {
                // Get the offset from the end before all 0s.
                let pre_zero = fract_str.bytes().rev().enumerate().find_map(|(i, b)| {
                    if b != b'0' {
                        Some(i)
                    } else {
                        None
                    }
                });

                if let Some(pre_zero) = pre_zero {
                    str.truncate(fract_end - pre_zero);
                } else {
                    str.truncate(dot_index);
                }
            }
        }
    }

    str
}

pub(crate) fn format_bytes(val: f64, sig_figs: usize, bytes_format: BytesFormat) -> String {
    let (val, scale) = scale_value(val, bytes_format);

    let mut result = format_f64(val, sig_figs);
    result.push(' ');
    result.push_str(scale.suffix(ScaleFormat::Bytes(bytes_format)));
    result
}

pub(crate) struct DisplayThroughput<'a> {
    pub counter: &'a AnyCounter,
    pub picos: f64,
    pub bytes_format: BytesFormat,
}

impl fmt::Debug for DisplayThroughput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for DisplayThroughput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let picos = self.picos;
        let count = self.counter.count();
        let count_per_sec = if count == 0 { 0. } else { count as f64 * (1e12 / picos) };

        let format = match self.counter.kind {
            KnownCounterKind::Bytes => ScaleFormat::BytesThroughput(self.bytes_format),
            KnownCounterKind::Chars => ScaleFormat::CharsThroughput,
            KnownCounterKind::Cycles => ScaleFormat::CyclesThroughput,
            KnownCounterKind::Items => ScaleFormat::ItemsThroughput,
        };

        let (val, scale) = scale_value(count_per_sec, format.bytes_format());

        let sig_figs = f.precision().unwrap_or(4);

        let mut str = format_f64(val, sig_figs);
        str.push(' ');
        str.push_str(scale.suffix(format));

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

/// Converts a value to the appropriate scale.
fn scale_value(value: f64, bytes_format: BytesFormat) -> (f64, Scale) {
    let starts = scale_starts(bytes_format);

    let scale = if value.is_infinite() || value < starts[1] {
        Scale::One
    } else if value < starts[2] {
        Scale::Kilo
    } else if value < starts[3] {
        Scale::Mega
    } else if value < starts[4] {
        Scale::Giga
    } else if value < starts[5] {
        Scale::Tera
    } else {
        Scale::Peta
    };

    (value / starts[scale as usize], scale)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scale {
    One,
    Kilo,
    Mega,
    Giga,
    Tera,
    Peta,
}

#[derive(Clone, Copy)]
pub(crate) enum ScaleFormat {
    Bytes(BytesFormat),
    BytesThroughput(BytesFormat),
    CharsThroughput,
    CyclesThroughput,
    ItemsThroughput,
}

impl ScaleFormat {
    pub fn bytes_format(self) -> BytesFormat {
        match self {
            Self::Bytes(format) | Self::BytesThroughput(format) => format,
            Self::CharsThroughput | Self::CyclesThroughput | Self::ItemsThroughput => {
                BytesFormat::Decimal
            }
        }
    }
}

fn scale_starts(bytes_format: BytesFormat) -> &'static [f64; Scale::COUNT] {
    const STARTS: &[[f64; Scale::COUNT]; 2] = &[
        [1., 1e3, 1e6, 1e9, 1e12, 1e15],
        [
            1.,
            1024.,
            1024u64.pow(2) as f64,
            1024u64.pow(3) as f64,
            1024u64.pow(4) as f64,
            1024u64.pow(5) as f64,
        ],
    ];

    &STARTS[bytes_format as usize]
}

impl Scale {
    const COUNT: usize = 6;

    pub fn suffix(self, format: ScaleFormat) -> &'static str {
        match format {
            ScaleFormat::Bytes(format) => {
                const SUFFIXES: &[[&str; Scale::COUNT]; 2] = &[
                    ["B", "KB", "MB", "GB", "TB", "PB"],
                    ["B", "KiB", "MiB", "GiB", "TiB", "PiB"],
                ];

                SUFFIXES[format as usize][self as usize]
            }
            ScaleFormat::BytesThroughput(format) => {
                const SUFFIXES: &[[&str; Scale::COUNT]; 2] = &[
                    ["B/s", "KB/s", "MB/s", "GB/s", "TB/s", "PB/s"],
                    ["B/s", "KiB/s", "MiB/s", "GiB/s", "TiB/s", "PiB/s"],
                ];

                SUFFIXES[format as usize][self as usize]
            }
            ScaleFormat::CharsThroughput => {
                const SUFFIXES: &[&str; Scale::COUNT] =
                    &["char/s", "Kchar/s", "Mchar/s", "Gchar/s", "Tchar/s", "Pchar/s"];

                SUFFIXES[self as usize]
            }
            ScaleFormat::CyclesThroughput => {
                const SUFFIXES: &[&str; Scale::COUNT] = &["Hz", "KHz", "MHz", "GHz", "THz", "PHz"];

                SUFFIXES[self as usize]
            }
            ScaleFormat::ItemsThroughput => {
                const SUFFIXES: &[&str; Scale::COUNT] =
                    &["item/s", "Kitem/s", "Mitem/s", "Gitem/s", "Titem/s", "Pitem/s"];

                SUFFIXES[self as usize]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_value() {
        #[track_caller]
        fn test(n: f64, format: BytesFormat, expected_value: f64, expected_scale: Scale) {
            assert_eq!(super::scale_value(n, format), (expected_value, expected_scale));
        }

        #[track_caller]
        fn test_decimal(n: f64, expected_value: f64, expected_scale: Scale) {
            test(n, BytesFormat::Decimal, expected_value, expected_scale);
        }

        test_decimal(1., 1., Scale::One);
        test_decimal(1_000., 1., Scale::Kilo);
        test_decimal(1_000_000., 1., Scale::Mega);
        test_decimal(1_000_000_000., 1., Scale::Giga);
        test_decimal(1_000_000_000_000., 1., Scale::Tera);
        test_decimal(1_000_000_000_000_000., 1., Scale::Peta);
    }
}
