use std::arch::asm;

use crate::time::TscUnavailable;

/// Reads the [`cntfrq_el0`](https://developer.arm.com/documentation/ddi0595/2021-12/AArch64-Registers/CNTFRQ-EL0--Counter-timer-Frequency-register?lang=en)
/// register.
///
/// This value is set on system initialization and thus does not change between
/// reads.
#[inline]
pub(crate) fn frequency() -> Result<u64, TscUnavailable> {
    unsafe {
        let frequency: u64;
        asm!(
            "mrs {}, cntfrq_el0",
            out(reg) frequency,
            options(nomem, nostack, preserves_flags, pure),
        );
        Ok(frequency)
    }
}

/// Reads the [`cntvct_el0`](https://developer.arm.com/documentation/ddi0595/2021-12/AArch64-Registers/CNTVCT-EL0--Counter-timer-Virtual-Count-register?lang=en)
/// register.
#[inline(always)]
pub(crate) fn timestamp() -> u64 {
    unsafe {
        let timestamp: u64;
        asm!(
            "mrs {}, cntvct_el0",
            out(reg) timestamp,
            // Leave off `nomem` because this should be a compiler fence.
            options(nostack, preserves_flags),
        );
        timestamp
    }
}
