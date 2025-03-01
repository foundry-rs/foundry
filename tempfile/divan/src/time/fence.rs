use std::sync::atomic;

/// Prevents other operations from affecting timing measurements.
#[inline(always)]
pub fn full_fence() {
    asm_fence();
    atomic::fence(atomic::Ordering::SeqCst);
}

/// Prevents the compiler from reordering operations.
#[inline(always)]
pub fn compiler_fence() {
    asm_fence();
    atomic::compiler_fence(atomic::Ordering::SeqCst);
}

/// Stronger compiler fence on [platforms with stable `asm!`](https://doc.rust-lang.org/nightly/reference/inline-assembly.html).
///
/// This prevents LLVM from removing loops or hoisting logic out of the
/// benchmark loop.
#[inline(always)]
fn asm_fence() {
    // Miri does not support inline assembly.
    if cfg!(miri) {
        return;
    }

    #[cfg(any(
        target_arch = "x86",
        target_arch = "x86_64",
        target_arch = "arm",
        target_arch = "aarch64",
        target_arch = "riscv32",
        target_arch = "riscv64",
        target_arch = "loongarch64",
    ))]
    // SAFETY: The inline assembly is a no-op.
    unsafe {
        // Preserve flags because we don't want to pessimize user logic.
        std::arch::asm!("", options(nostack, preserves_flags));
    }
}
