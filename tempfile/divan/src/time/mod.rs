use std::time::Duration;

pub mod fence;

mod fine_duration;
mod timer;
mod timestamp;

pub(crate) use fine_duration::*;
pub(crate) use timer::*;
pub(crate) use timestamp::*;

/// Private-public trait for being polymorphic over `Duration`.
pub trait IntoDuration {
    /// Converts into a `Duration`.
    fn into_duration(self) -> Duration;
}

impl IntoDuration for Duration {
    #[inline]
    fn into_duration(self) -> Duration {
        self
    }
}

impl IntoDuration for u64 {
    #[inline]
    fn into_duration(self) -> Duration {
        Duration::from_secs(self)
    }
}

impl IntoDuration for f64 {
    #[inline]
    fn into_duration(self) -> Duration {
        Duration::from_secs_f64(self)
    }
}
