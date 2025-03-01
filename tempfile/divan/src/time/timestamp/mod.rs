use std::time::Instant;

use crate::time::{fence, FineDuration, Timer, TimerKind};

mod tsc;

pub(crate) use tsc::*;

/// A measurement timestamp.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Timestamp {
    /// Time provided by the operating system.
    Os(Instant),

    /// [CPU timestamp counter](https://en.wikipedia.org/wiki/Time_Stamp_Counter).
    Tsc(TscTimestamp),
}

impl Timestamp {
    #[inline(always)]
    pub fn start(timer_kind: TimerKind) -> Self {
        fence::full_fence();
        let value = match timer_kind {
            TimerKind::Os => Self::Os(Instant::now()),
            TimerKind::Tsc => Self::Tsc(TscTimestamp::start()),
        };
        fence::compiler_fence();
        value
    }

    pub fn duration_since(self, earlier: Self, timer: Timer) -> FineDuration {
        match (self, earlier, timer) {
            (Self::Os(this), Self::Os(earlier), Timer::Os) => this.duration_since(earlier).into(),
            (Self::Tsc(this), Self::Tsc(earlier), Timer::Tsc { frequency }) => {
                this.duration_since(earlier, frequency)
            }
            _ => unreachable!(),
        }
    }
}

/// A [`Timestamp`] where the variant is determined by an external source of
/// truth.
///
/// By making the variant tag external to this type, we produce more optimized
/// code by:
/// - Reusing the same condition variable
/// - Reducing the size of the timestamp variables
#[derive(Clone, Copy)]
pub(crate) union UntaggedTimestamp {
    /// [`Timestamp::Os`].
    pub os: Instant,

    /// [`Timestamp::Tsc`].
    pub tsc: TscTimestamp,
}

impl UntaggedTimestamp {
    #[inline(always)]
    pub fn start(timer_kind: TimerKind) -> Self {
        fence::full_fence();
        let value = match timer_kind {
            TimerKind::Os => Self { os: Instant::now() },
            TimerKind::Tsc => Self { tsc: TscTimestamp::start() },
        };
        fence::compiler_fence();
        value
    }

    #[inline(always)]
    pub fn end(timer_kind: TimerKind) -> Self {
        fence::compiler_fence();
        let value = match timer_kind {
            TimerKind::Os => Self { os: Instant::now() },
            TimerKind::Tsc => Self { tsc: TscTimestamp::end() },
        };
        fence::full_fence();
        value
    }

    #[inline(always)]
    pub unsafe fn into_timestamp(self, timer_kind: TimerKind) -> Timestamp {
        match timer_kind {
            TimerKind::Os => Timestamp::Os(self.os),
            TimerKind::Tsc => Timestamp::Tsc(self.tsc),
        }
    }
}
