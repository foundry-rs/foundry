//! Formatting for log records.
//!
//! This module contains a [`Formatter`] that can be used to format log records
//! into without needing temporary allocations. Usually you won't need to worry
//! about the contents of this module and can use the `Formatter` like an ordinary
//! [`Write`].
//!
//! # Formatting log records
//!
//! The format used to print log records can be customised using the [`Builder::format`]
//! method.
//!
//! Terminal styling is done through ANSI escape codes and will be adapted to the capabilities of
//! the target stream.s
//!
//! For example, you could use one of:
//! - [anstyle](https://docs.rs/anstyle) is a minimal, runtime string styling API and is re-exported as [`style`]
//! - [owo-colors](https://docs.rs/owo-colors) is a feature rich runtime string styling API
//! - [color-print](https://docs.rs/color-print) for feature-rich compile-time styling API
//!
//! See also [`Formatter::default_level_style`]
//!
//! ```
//! use std::io::Write;
//!
//! let mut builder = env_logger::Builder::new();
//!
//! builder.format(|buf, record| {
//!     writeln!(buf, "{}: {}",
//!         record.level(),
//!         record.args())
//! });
//! ```
//!
//! # Key Value arguments
//!
//! If the `unstable-kv` feature is enabled, then the default format will include key values from
//! the log by default, but this can be disabled by calling [`Builder::format_key_values`]
//! with [`hidden_kv_format`] as the format function.
//!
//! The way these keys and values are formatted can also be customized with a separate format
//! function that is called by the default format with [`Builder::format_key_values`].
//!
//! ```
//! # #[cfg(feature= "unstable-kv")]
//! # {
//! use log::info;
//! env_logger::init();
//! info!(x="45"; "Some message");
//! info!(x="12"; "Another message {x}", x="12");
//! # }
//! ```
//!
//! See <https://docs.rs/log/latest/log/#structured-logging>.
//!
//! [`Builder::format`]: crate::Builder::format
//! [`Write`]: std::io::Write
//! [`Builder::format_key_values`]: crate::Builder::format_key_values

use std::cell::RefCell;
use std::fmt::Display;
use std::io::prelude::Write;
use std::rc::Rc;
use std::{fmt, io, mem};

#[cfg(feature = "color")]
use log::Level;
use log::Record;

#[cfg(feature = "humantime")]
mod humantime;
#[cfg(feature = "unstable-kv")]
mod kv;
pub(crate) mod writer;

#[cfg(feature = "color")]
pub use anstyle as style;

#[cfg(feature = "humantime")]
pub use self::humantime::Timestamp;
#[cfg(feature = "unstable-kv")]
pub use self::kv::*;
pub use self::writer::Target;
pub use self::writer::WriteStyle;

use self::writer::{Buffer, Writer};

/// Formatting precision of timestamps.
///
/// Seconds give precision of full seconds, milliseconds give thousands of a
/// second (3 decimal digits), microseconds are millionth of a second (6 decimal
/// digits) and nanoseconds are billionth of a second (9 decimal digits).
#[allow(clippy::exhaustive_enums)] // compatibility
#[derive(Copy, Clone, Debug)]
pub enum TimestampPrecision {
    /// Full second precision (0 decimal digits)
    Seconds,
    /// Millisecond precision (3 decimal digits)
    Millis,
    /// Microsecond precision (6 decimal digits)
    Micros,
    /// Nanosecond precision (9 decimal digits)
    Nanos,
}

/// The default timestamp precision is seconds.
impl Default for TimestampPrecision {
    fn default() -> Self {
        TimestampPrecision::Seconds
    }
}

/// A formatter to write logs into.
///
/// `Formatter` implements the standard [`Write`] trait for writing log records.
/// It also supports terminal styling using ANSI escape codes.
///
/// # Examples
///
/// Use the [`writeln`] macro to format a log record.
/// An instance of a `Formatter` is passed to an `env_logger` format as `buf`:
///
/// ```
/// use std::io::Write;
///
/// let mut builder = env_logger::Builder::new();
///
/// builder.format(|buf, record| writeln!(buf, "{}: {}", record.level(), record.args()));
/// ```
///
/// [`Write`]: std::io::Write
/// [`writeln`]: std::writeln
pub struct Formatter {
    buf: Rc<RefCell<Buffer>>,
    write_style: WriteStyle,
}

impl Formatter {
    pub(crate) fn new(writer: &Writer) -> Self {
        Formatter {
            buf: Rc::new(RefCell::new(writer.buffer())),
            write_style: writer.write_style(),
        }
    }

    pub(crate) fn write_style(&self) -> WriteStyle {
        self.write_style
    }

    pub(crate) fn print(&self, writer: &Writer) -> io::Result<()> {
        writer.print(&self.buf.borrow())
    }

    pub(crate) fn clear(&mut self) {
        self.buf.borrow_mut().clear();
    }
}

#[cfg(feature = "color")]
impl Formatter {
    /// Get the default [`style::Style`] for the given level.
    ///
    /// The style can be used to print other values besides the level.
    ///
    /// See [`style`] for how to adapt it to the styling crate of your choice
    pub fn default_level_style(&self, level: Level) -> style::Style {
        if self.write_style == WriteStyle::Never {
            style::Style::new()
        } else {
            match level {
                Level::Trace => style::AnsiColor::Cyan.on_default(),
                Level::Debug => style::AnsiColor::Blue.on_default(),
                Level::Info => style::AnsiColor::Green.on_default(),
                Level::Warn => style::AnsiColor::Yellow.on_default(),
                Level::Error => style::AnsiColor::Red
                    .on_default()
                    .effects(style::Effects::BOLD),
            }
        }
    }
}

impl Write for Formatter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.borrow_mut().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.buf.borrow_mut().flush()
    }
}

impl fmt::Debug for Formatter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let buf = self.buf.borrow();
        f.debug_struct("Formatter")
            .field("buf", &buf)
            .field("write_style", &self.write_style)
            .finish()
    }
}

pub(crate) type FormatFn = Box<dyn Fn(&mut Formatter, &Record<'_>) -> io::Result<()> + Sync + Send>;

pub(crate) struct Builder {
    pub(crate) format_timestamp: Option<TimestampPrecision>,
    pub(crate) format_module_path: bool,
    pub(crate) format_target: bool,
    pub(crate) format_level: bool,
    pub(crate) format_indent: Option<usize>,
    pub(crate) custom_format: Option<FormatFn>,
    pub(crate) format_suffix: &'static str,
    pub(crate) format_file: bool,
    pub(crate) format_line_number: bool,
    #[cfg(feature = "unstable-kv")]
    pub(crate) kv_format: Option<Box<KvFormatFn>>,
    built: bool,
}

impl Builder {
    /// Convert the format into a callable function.
    ///
    /// If the `custom_format` is `Some`, then any `default_format` switches are ignored.
    /// If the `custom_format` is `None`, then a default format is returned.
    /// Any `default_format` switches set to `false` won't be written by the format.
    pub(crate) fn build(&mut self) -> FormatFn {
        assert!(!self.built, "attempt to re-use consumed builder");

        let built = mem::replace(
            self,
            Builder {
                built: true,
                ..Default::default()
            },
        );

        if let Some(fmt) = built.custom_format {
            fmt
        } else {
            Box::new(move |buf, record| {
                let fmt = DefaultFormat {
                    timestamp: built.format_timestamp,
                    module_path: built.format_module_path,
                    target: built.format_target,
                    level: built.format_level,
                    written_header_value: false,
                    indent: built.format_indent,
                    suffix: built.format_suffix,
                    source_file: built.format_file,
                    source_line_number: built.format_line_number,
                    #[cfg(feature = "unstable-kv")]
                    kv_format: built.kv_format.as_deref().unwrap_or(&default_kv_format),
                    buf,
                };

                fmt.write(record)
            })
        }
    }
}

impl Default for Builder {
    fn default() -> Self {
        Builder {
            format_timestamp: Some(Default::default()),
            format_module_path: false,
            format_target: true,
            format_level: true,
            format_file: false,
            format_line_number: false,
            format_indent: Some(4),
            custom_format: None,
            format_suffix: "\n",
            #[cfg(feature = "unstable-kv")]
            kv_format: None,
            built: false,
        }
    }
}

#[cfg(feature = "color")]
type SubtleStyle = StyledValue<&'static str>;
#[cfg(not(feature = "color"))]
type SubtleStyle = &'static str;

/// A value that can be printed using the given styles.
#[cfg(feature = "color")]
struct StyledValue<T> {
    style: style::Style,
    value: T,
}

#[cfg(feature = "color")]
impl<T: Display> Display for StyledValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let style = self.style;

        // We need to make sure `f`s settings don't get passed onto the styling but do get passed
        // to the value
        write!(f, "{style}")?;
        self.value.fmt(f)?;
        write!(f, "{style:#}")?;
        Ok(())
    }
}

#[cfg(not(feature = "color"))]
type StyledValue<T> = T;

/// The default format.
///
/// This format needs to work with any combination of crate features.
struct DefaultFormat<'a> {
    timestamp: Option<TimestampPrecision>,
    module_path: bool,
    target: bool,
    level: bool,
    source_file: bool,
    source_line_number: bool,
    written_header_value: bool,
    indent: Option<usize>,
    buf: &'a mut Formatter,
    suffix: &'a str,
    #[cfg(feature = "unstable-kv")]
    kv_format: &'a KvFormatFn,
}

impl DefaultFormat<'_> {
    fn write(mut self, record: &Record<'_>) -> io::Result<()> {
        self.write_timestamp()?;
        self.write_level(record)?;
        self.write_module_path(record)?;
        self.write_source_location(record)?;
        self.write_target(record)?;
        self.finish_header()?;

        self.write_args(record)?;
        #[cfg(feature = "unstable-kv")]
        self.write_kv(record)?;
        write!(self.buf, "{}", self.suffix)
    }

    fn subtle_style(&self, text: &'static str) -> SubtleStyle {
        #[cfg(feature = "color")]
        {
            StyledValue {
                style: if self.buf.write_style == WriteStyle::Never {
                    style::Style::new()
                } else {
                    style::AnsiColor::BrightBlack.on_default()
                },
                value: text,
            }
        }
        #[cfg(not(feature = "color"))]
        {
            text
        }
    }

    fn write_header_value<T>(&mut self, value: T) -> io::Result<()>
    where
        T: Display,
    {
        if !self.written_header_value {
            self.written_header_value = true;

            let open_brace = self.subtle_style("[");
            write!(self.buf, "{open_brace}{value}")
        } else {
            write!(self.buf, " {value}")
        }
    }

    fn write_level(&mut self, record: &Record<'_>) -> io::Result<()> {
        if !self.level {
            return Ok(());
        }

        let level = {
            let level = record.level();
            #[cfg(feature = "color")]
            {
                StyledValue {
                    style: self.buf.default_level_style(level),
                    value: level,
                }
            }
            #[cfg(not(feature = "color"))]
            {
                level
            }
        };

        self.write_header_value(format_args!("{level:<5}"))
    }

    fn write_timestamp(&mut self) -> io::Result<()> {
        #[cfg(feature = "humantime")]
        {
            use self::TimestampPrecision::{Micros, Millis, Nanos, Seconds};
            let ts = match self.timestamp {
                None => return Ok(()),
                Some(Seconds) => self.buf.timestamp_seconds(),
                Some(Millis) => self.buf.timestamp_millis(),
                Some(Micros) => self.buf.timestamp_micros(),
                Some(Nanos) => self.buf.timestamp_nanos(),
            };

            self.write_header_value(ts)
        }
        #[cfg(not(feature = "humantime"))]
        {
            // Trick the compiler to think we have used self.timestamp
            // Workaround for "field is never used: `timestamp`" compiler nag.
            let _ = self.timestamp;
            Ok(())
        }
    }

    fn write_module_path(&mut self, record: &Record<'_>) -> io::Result<()> {
        if !self.module_path {
            return Ok(());
        }

        if let Some(module_path) = record.module_path() {
            self.write_header_value(module_path)
        } else {
            Ok(())
        }
    }

    fn write_source_location(&mut self, record: &Record<'_>) -> io::Result<()> {
        if !self.source_file {
            return Ok(());
        }

        if let Some(file_path) = record.file() {
            let line = self.source_line_number.then(|| record.line()).flatten();
            match line {
                Some(line) => self.write_header_value(format_args!("{file_path}:{line}")),
                None => self.write_header_value(file_path),
            }
        } else {
            Ok(())
        }
    }

    fn write_target(&mut self, record: &Record<'_>) -> io::Result<()> {
        if !self.target {
            return Ok(());
        }

        match record.target() {
            "" => Ok(()),
            target => self.write_header_value(target),
        }
    }

    fn finish_header(&mut self) -> io::Result<()> {
        if self.written_header_value {
            let close_brace = self.subtle_style("]");
            write!(self.buf, "{close_brace} ")
        } else {
            Ok(())
        }
    }

    fn write_args(&mut self, record: &Record<'_>) -> io::Result<()> {
        match self.indent {
            // Fast path for no indentation
            None => write!(self.buf, "{}", record.args()),

            Some(indent_count) => {
                // Create a wrapper around the buffer only if we have to actually indent the message

                struct IndentWrapper<'a, 'b> {
                    fmt: &'a mut DefaultFormat<'b>,
                    indent_count: usize,
                }

                impl Write for IndentWrapper<'_, '_> {
                    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                        let mut first = true;
                        for chunk in buf.split(|&x| x == b'\n') {
                            if !first {
                                write!(
                                    self.fmt.buf,
                                    "{}{:width$}",
                                    self.fmt.suffix,
                                    "",
                                    width = self.indent_count
                                )?;
                            }
                            self.fmt.buf.write_all(chunk)?;
                            first = false;
                        }

                        Ok(buf.len())
                    }

                    fn flush(&mut self) -> io::Result<()> {
                        self.fmt.buf.flush()
                    }
                }

                // The explicit scope here is just to make older versions of Rust happy
                {
                    let mut wrapper = IndentWrapper {
                        fmt: self,
                        indent_count,
                    };
                    write!(wrapper, "{}", record.args())?;
                }

                Ok(())
            }
        }
    }

    #[cfg(feature = "unstable-kv")]
    fn write_kv(&mut self, record: &Record<'_>) -> io::Result<()> {
        let format = self.kv_format;
        format(self.buf, record.key_values())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use log::{Level, Record};

    fn write_record(record: Record<'_>, fmt: DefaultFormat<'_>) -> String {
        let buf = fmt.buf.buf.clone();

        fmt.write(&record).expect("failed to write record");

        let buf = buf.borrow();
        String::from_utf8(buf.as_bytes().to_vec()).expect("failed to read record")
    }

    fn write_target(target: &str, fmt: DefaultFormat<'_>) -> String {
        write_record(
            Record::builder()
                .args(format_args!("log\nmessage"))
                .level(Level::Info)
                .file(Some("test.rs"))
                .line(Some(144))
                .module_path(Some("test::path"))
                .target(target)
                .build(),
            fmt,
        )
    }

    fn write(fmt: DefaultFormat<'_>) -> String {
        write_target("", fmt)
    }

    fn formatter() -> Formatter {
        let writer = writer::Builder::new()
            .write_style(WriteStyle::Never)
            .build();

        Formatter::new(&writer)
    }

    #[test]
    fn format_with_header() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: true,
            target: false,
            level: true,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: None,
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("[INFO  test::path] log\nmessage\n", written);
    }

    #[test]
    fn format_no_header() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: false,
            target: false,
            level: false,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: None,
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("log\nmessage\n", written);
    }

    #[test]
    fn format_indent_spaces() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: true,
            target: false,
            level: true,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: Some(4),
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("[INFO  test::path] log\n    message\n", written);
    }

    #[test]
    fn format_indent_zero_spaces() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: true,
            target: false,
            level: true,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: Some(0),
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("[INFO  test::path] log\nmessage\n", written);
    }

    #[test]
    fn format_indent_spaces_no_header() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: false,
            target: false,
            level: false,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: Some(4),
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("log\n    message\n", written);
    }

    #[test]
    fn format_suffix() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: false,
            target: false,
            level: false,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: None,
            suffix: "\n\n",
            buf: &mut f,
        });

        assert_eq!("log\nmessage\n\n", written);
    }

    #[test]
    fn format_suffix_with_indent() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: false,
            target: false,
            level: false,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: Some(4),
            suffix: "\n\n",
            buf: &mut f,
        });

        assert_eq!("log\n\n    message\n\n", written);
    }

    #[test]
    fn format_target() {
        let mut f = formatter();

        let written = write_target(
            "target",
            DefaultFormat {
                timestamp: None,
                module_path: true,
                target: true,
                level: true,
                source_file: false,
                source_line_number: false,
                #[cfg(feature = "unstable-kv")]
                kv_format: &hidden_kv_format,
                written_header_value: false,
                indent: None,
                suffix: "\n",
                buf: &mut f,
            },
        );

        assert_eq!("[INFO  test::path target] log\nmessage\n", written);
    }

    #[test]
    fn format_empty_target() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: true,
            target: true,
            level: true,
            source_file: false,
            source_line_number: false,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: None,
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("[INFO  test::path] log\nmessage\n", written);
    }

    #[test]
    fn format_no_target() {
        let mut f = formatter();

        let written = write_target(
            "target",
            DefaultFormat {
                timestamp: None,
                module_path: true,
                target: false,
                level: true,
                source_file: false,
                source_line_number: false,
                #[cfg(feature = "unstable-kv")]
                kv_format: &hidden_kv_format,
                written_header_value: false,
                indent: None,
                suffix: "\n",
                buf: &mut f,
            },
        );

        assert_eq!("[INFO  test::path] log\nmessage\n", written);
    }

    #[test]
    fn format_with_source_file_and_line_number() {
        let mut f = formatter();

        let written = write(DefaultFormat {
            timestamp: None,
            module_path: false,
            target: false,
            level: true,
            source_file: true,
            source_line_number: true,
            #[cfg(feature = "unstable-kv")]
            kv_format: &hidden_kv_format,
            written_header_value: false,
            indent: None,
            suffix: "\n",
            buf: &mut f,
        });

        assert_eq!("[INFO  test.rs:144] log\nmessage\n", written);
    }

    #[cfg(feature = "unstable-kv")]
    #[test]
    fn format_kv_default() {
        let kvs = &[("a", 1u32), ("b", 2u32)][..];
        let mut f = formatter();
        let record = Record::builder()
            .args(format_args!("log message"))
            .level(Level::Info)
            .module_path(Some("test::path"))
            .key_values(&kvs)
            .build();

        let written = write_record(
            record,
            DefaultFormat {
                timestamp: None,
                module_path: false,
                target: false,
                level: true,
                source_file: false,
                source_line_number: false,
                kv_format: &default_kv_format,
                written_header_value: false,
                indent: None,
                suffix: "\n",
                buf: &mut f,
            },
        );

        assert_eq!("[INFO ] log message a=1 b=2\n", written);
    }

    #[cfg(feature = "unstable-kv")]
    #[test]
    fn format_kv_default_full() {
        let kvs = &[("a", 1u32), ("b", 2u32)][..];
        let mut f = formatter();
        let record = Record::builder()
            .args(format_args!("log\nmessage"))
            .level(Level::Info)
            .module_path(Some("test::path"))
            .target("target")
            .file(Some("test.rs"))
            .line(Some(42))
            .key_values(&kvs)
            .build();

        let written = write_record(
            record,
            DefaultFormat {
                timestamp: None,
                module_path: true,
                target: true,
                level: true,
                source_file: true,
                source_line_number: true,
                kv_format: &default_kv_format,
                written_header_value: false,
                indent: None,
                suffix: "\n",
                buf: &mut f,
            },
        );

        assert_eq!(
            "[INFO  test::path test.rs:42 target] log\nmessage a=1 b=2\n",
            written
        );
    }
}
