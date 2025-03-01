//! Backtrace strategy for MSVC platforms.
//!
//! This module contains the ability to capture a backtrace on MSVC using one
//! of three possible methods. For `x86_64` and `aarch64`, we use `RtlVirtualUnwind`
//! to walk the stack one frame at a time. This function is much faster than using
//! `dbghelp!StackWalk*` because it does not load debug info to report inlined frames.
//! We still report inlined frames during symbolization by consulting the appropriate
//! `dbghelp` functions.
//!
//! For all other platforms, primarily `i686`, the `StackWalkEx` function is used if
//! possible, but not all systems have that. Failing that the `StackWalk64` function
//! is used instead. Note that `StackWalkEx` is favored because it handles debuginfo
//! internally and returns inline frame information.
//!
//! Note that all dbghelp support is loaded dynamically, see `src/dbghelp.rs`
//! for more information about that.

#![allow(bad_style)]

use super::super::windows::*;
use core::ffi::c_void;

#[derive(Clone, Copy)]
pub struct Frame {
    base_address: *mut c_void,
    ip: *mut c_void,
    sp: *mut c_void,
    #[cfg(not(target_env = "gnu"))]
    inline_context: Option<DWORD>,
}

// we're just sending around raw pointers and reading them, never interpreting
// them so this should be safe to both send and share across threads.
unsafe impl Send for Frame {}
unsafe impl Sync for Frame {}

impl Frame {
    pub fn ip(&self) -> *mut c_void {
        self.ip
    }

    pub fn sp(&self) -> *mut c_void {
        self.sp
    }

    pub fn symbol_address(&self) -> *mut c_void {
        self.ip
    }

    pub fn module_base_address(&self) -> Option<*mut c_void> {
        Some(self.base_address)
    }

    #[cfg(not(target_env = "gnu"))]
    pub fn inline_context(&self) -> Option<DWORD> {
        self.inline_context
    }
}

#[repr(C, align(16))] // required by `CONTEXT`, is a FIXME in winapi right now
struct MyContext(CONTEXT);

#[cfg(any(target_arch = "x86_64", target_arch = "arm64ec"))]
impl MyContext {
    #[inline(always)]
    fn ip(&self) -> DWORD64 {
        self.0.Rip
    }

    #[inline(always)]
    fn sp(&self) -> DWORD64 {
        self.0.Rsp
    }
}

#[cfg(target_arch = "aarch64")]
impl MyContext {
    #[inline(always)]
    fn ip(&self) -> DWORD64 {
        self.0.Pc
    }

    #[inline(always)]
    fn sp(&self) -> DWORD64 {
        self.0.Sp
    }
}

#[cfg(any(
    target_arch = "x86_64",
    target_arch = "aarch64",
    target_arch = "arm64ec"
))]
#[inline(always)]
pub unsafe fn trace(cb: &mut dyn FnMut(&super::Frame) -> bool) {
    use core::ptr;

    let mut context = core::mem::zeroed::<MyContext>();
    RtlCaptureContext(&mut context.0);

    // Call `RtlVirtualUnwind` to find the previous stack frame, walking until we hit ip = 0.
    while context.ip() != 0 {
        let mut base = 0;

        let fn_entry = RtlLookupFunctionEntry(context.ip(), &mut base, ptr::null_mut());
        if fn_entry.is_null() {
            break;
        }

        let frame = super::Frame {
            inner: Frame {
                base_address: fn_entry.cast::<c_void>(),
                ip: context.ip() as *mut c_void,
                sp: context.sp() as *mut c_void,
                #[cfg(not(target_env = "gnu"))]
                inline_context: None,
            },
        };

        if !cb(&frame) {
            break;
        }

        let mut handler_data = 0usize;
        let mut establisher_frame = 0;

        RtlVirtualUnwind(
            0,
            base,
            context.ip(),
            fn_entry,
            &mut context.0,
            ptr::addr_of_mut!(handler_data).cast::<PVOID>(),
            &mut establisher_frame,
            ptr::null_mut(),
        );
    }
}
