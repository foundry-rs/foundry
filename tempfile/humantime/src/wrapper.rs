use std::str::FromStr;
use std::ops::Deref;
use std::fmt;
use std::time::{Duration as StdDuration, SystemTime};

use crate::duration::{self, parse_duration, format_duration};
use crate::date::{self, parse_rfc3339_weak, format_rfc3339};

/// A wrapper for duration that has `FromStr` implementation
///
/// This is useful if you want to use it somewhere where `FromStr` is
/// expected.
///
/// See `parse_duration` for the description of the format.
///
/// # Example
///
/// ```
/// use std::time::Duration;
/// let x: Duration;
/// x = "12h 5min 2ns".parse::<humantime::Duration>().unwrap().into();
/// assert_eq!(x, Duration::new(12*3600 + 5*60, 2))
/// ```
///
#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub struct Duration(StdDuration);

/// A wrapper for SystemTime that has `FromStr` implementation
///
/// This is useful if you want to use it somewhere where `FromStr` is
/// expected.
///
/// See `parse_rfc3339_weak` for the description of the format. The "weak"
/// format is used as it's more pemissive for human input as this is the
/// expected use of the type (e.g. command-line parsing).
///
/// # Example
///
/// ```
/// use std::time::SystemTime;
/// let x: SystemTime;
/// x = "2018-02-16T00:31:37Z".parse::<humantime::Timestamp>().unwrap().into();
/// assert_eq!(humantime::format_rfc3339(x).to_string(), "2018-02-16T00:31:37Z");
/// ```
///
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct Timestamp(SystemTime);

impl AsRef<StdDuration> for Duration {
    fn as_ref(&self) -> &StdDuration { &self.0 }
}

impl Deref for Duration {
    type Target = StdDuration;
    fn deref(&self) -> &StdDuration { &self.0 }
}

impl Into<StdDuration> for Duration {
    fn into(self) -> StdDuration { self.0 }
}

impl From<StdDuration> for Duration {
    fn from(dur: StdDuration) -> Duration { Duration(dur) }
}

impl FromStr for Duration {
    type Err = duration::Error;
    fn from_str(s: &str) -> Result<Duration, Self::Err> {
        parse_duration(s).map(Duration)
    }
}

impl fmt::Display for Duration {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_duration(self.0).fmt(f)
    }
}

impl AsRef<SystemTime> for Timestamp {
    fn as_ref(&self) -> &SystemTime { &self.0 }
}

impl Deref for Timestamp {
    type Target = SystemTime;
    fn deref(&self) -> &SystemTime { &self.0 }
}

impl Into<SystemTime> for Timestamp {
    fn into(self) -> SystemTime { self.0 }
}

impl From<SystemTime> for Timestamp {
    fn from(dur: SystemTime) -> Timestamp { Timestamp(dur) }
}

impl FromStr for Timestamp {
    type Err = date::Error;
    fn from_str(s: &str) -> Result<Timestamp, Self::Err> {
        parse_rfc3339_weak(s).map(Timestamp)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        format_rfc3339(self.0).fmt(f)
    }
}
