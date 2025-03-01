use std::{fmt, num::NonZeroU64};

use crate::time::FineDuration;

#[cfg(target_arch = "aarch64")]
#[path = "aarch64.rs"]
mod arch;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[path = "x86.rs"]
mod arch;

/// [CPU timestamp counter](https://en.wikipedia.org/wiki/Time_Stamp_Counter).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub(crate) struct TscTimestamp {
    pub value: u64,
}

impl TscTimestamp {
    /// Gets the timestamp frequency.
    ///
    /// On AArch64, this simply reads `cntfrq_el0`. On x86, this measures the
    /// TSC frequency.
    #[inline]
    #[allow(unreachable_code)]
    pub fn frequency() -> Result<NonZeroU64, TscUnavailable> {
        // Miri does not support inline assembly.
        #[cfg(miri)]
        return Err(TscUnavailable::Unimplemented);

        #[cfg(any(target_arch = "aarch64", target_arch = "x86", target_arch = "x86_64"))]
        return NonZeroU64::new(arch::frequency()?).ok_or(TscUnavailable::ZeroFrequency);

        Err(TscUnavailable::Unimplemented)
    }

    /// Reads the timestamp counter.
    #[inline(always)]
    pub fn start() -> Self {
        #[allow(unused)]
        let value = 0;

        #[cfg(target_arch = "aarch64")]
        let value = arch::timestamp();

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        let value = arch::start_timestamp();

        Self { value }
    }

    /// Reads the timestamp counter.
    #[inline(always)]
    pub fn end() -> Self {
        #[allow(unused)]
        let value = 0;

        #[cfg(target_arch = "aarch64")]
        let value = arch::timestamp();

        #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
        let value = arch::end_timestamp();

        Self { value }
    }

    pub fn duration_since(self, earlier: Self, frequency: NonZeroU64) -> FineDuration {
        const PICOS: u128 = 1_000_000_000_000;

        let Some(diff) = self.value.checked_sub(earlier.value) else {
            return Default::default();
        };

        FineDuration { picos: (diff as u128 * PICOS) / frequency.get() as u128 }
    }
}

/// Reason for why the timestamp counter cannot be used.
#[derive(Clone, Copy)]
pub(crate) enum TscUnavailable {
    /// Not yet implemented for this platform.
    Unimplemented,

    /// Got a frequency of 0.
    ZeroFrequency,

    /// Missing the appropriate instructions.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    MissingInstructions,

    /// The timestamp counter is not guaranteed to be constant.
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    VariableFrequency,
}

impl fmt::Display for TscUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let reason = match self {
            Self::Unimplemented => "unimplemented",
            Self::ZeroFrequency => "zero TSC frequency",

            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::MissingInstructions => "missing instructions",

            #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
            Self::VariableFrequency => "variable TSC frequency",
        };

        f.write_str(reason)
    }
}
