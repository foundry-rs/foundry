use crate::{Category, MessageBuf, ErrorCode};
use crate::utils::write_fallback_code;
use crate::types::c_int;

use core::{ptr, str};

/// Posix error category, suitable for all environments.
///
/// In presence of OS, it means it identifies POSIX error codes.
pub static POSIX_CATEGORY: Category = Category {
    name: "PosixError",
    message,
    equivalent,
    is_would_block,
};

fn equivalent(code: c_int, other: &ErrorCode) -> bool {
    ptr::eq(&POSIX_CATEGORY, other.category()) && code == other.raw_code()
}

#[cfg(not(any(target_os = "cloudabi", target_os = "unknown")))]
pub(crate) fn get_last_error() -> c_int {
    //Reference:
    //https://github.com/rust-lang/rust/blob/2ae1bb671183a072b54ed8ed39abfcd72990a3e7/library/std/src/sys/pal/unix/os.rs#L42
    extern {
        #[cfg(not(any(target_os = "dragonfly", target_os = "vxworks")))]
        #[cfg_attr(
            any(
                target_os = "linux",
                target_os = "emscripten",
                target_os = "fuchsia",
                target_os = "l4re",
                target_os = "hurd",
                target_os = "teeos",
                target_os = "wasi"
            ),
            link_name = "__errno_location"
        )]
        #[cfg_attr(
            any(
                target_os = "netbsd",
                target_os = "openbsd",
                target_os = "android",
                target_os = "redox",
                target_env = "newlib"
            ),
            link_name = "__errno"
        )]
        #[cfg_attr(any(target_os = "solaris", target_os = "illumos"), link_name = "___errno")]
        #[cfg_attr(target_os = "nto", link_name = "__get_errno_ptr")]
        #[cfg_attr(
            any(
                target_os = "macos",
                target_os = "ios",
                target_os = "tvos",
                target_os = "freebsd",
                target_os = "watchos"
            ),
            link_name = "__error"
        )]
        #[cfg_attr(target_os = "haiku", link_name = "_errnop")]
        #[cfg_attr(target_os = "aix", link_name = "_Errno")]
        #[cfg_attr(target_os = "windows", link_name = "_errno")]
        fn errno_location() -> *mut c_int;
    }

    unsafe {
        *(errno_location())
    }
}

#[cfg(any(target_os = "cloudabi", target_os = "dragonfly"))]
pub(crate) fn get_last_error() -> c_int {
    //WASI implements it as thread local, but thread local are not stable :(
    extern {
        #[thread_local]
        static errno: c_int;
    }

    errno
}

#[cfg(target_os = "vxworks")]
pub(crate) fn get_last_error() -> c_int {
    extern "C" {
        pub fn errnoGet() -> c_int;
    }

    unsafe {
        errnoGet()
    }
}

#[cfg(all(target_os = "unknown", not(target_env = "newlib")))]
pub(crate) fn get_last_error() -> c_int {
    0
}

pub(crate) fn message(_code: c_int, out: &mut MessageBuf) -> &str {
    #[cfg(any(windows, target_os = "wasi", all(unix, not(target_env = "gnu"))))]
    extern "C" {
        ///Only GNU impl is thread unsafe
        fn strerror(code: c_int) -> *const i8;
        fn strlen(text: *const i8) -> usize;
    }

    #[cfg(all(unix, target_env = "gnu"))]
    extern "C" {
        fn strerror_l(code: c_int, locale: *mut i8) -> *const i8;
        fn strlen(text: *const i8) -> usize;
    }

    #[cfg(all(unix, target_env = "gnu"))]
    #[inline]
    unsafe fn strerror(code: c_int) -> *const i8 {
        strerror_l(code, ptr::null_mut())
    }

    #[cfg(any(windows, unix, target_os = "wasi"))]
    {
        let err = unsafe {
            strerror(_code)
        };

        if !err.is_null() {
            let err_len = unsafe {
                core::cmp::min(out.len(), strlen(err) as usize)
            };

            let err_slice = unsafe {
                ptr::copy_nonoverlapping(err as *const u8, out.as_mut_ptr() as *mut u8, err_len);
                core::slice::from_raw_parts(out.as_ptr() as *const u8, err_len)
            };

            if let Ok(msg) = str::from_utf8(err_slice) {
                return msg
            }
        }
    }

    write_fallback_code(out, _code)
}

#[cfg(not(any(windows, unix, target_os = "wasi")))]
pub(crate) fn is_would_block(_: c_int) -> bool {
    false
}

#[cfg(any(windows, unix, target_os = "wasi"))]
pub(crate) fn is_would_block(code: c_int) -> bool {
    code == crate::defs::EWOULDBLOCK || code == crate::defs::EAGAIN
}
