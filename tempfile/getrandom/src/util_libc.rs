use crate::Error;
use core::mem::MaybeUninit;

cfg_if! {
    if #[cfg(any(target_os = "netbsd", target_os = "openbsd", target_os = "android"))] {
        use libc::__errno as errno_location;
    } else if #[cfg(any(target_os = "linux", target_os = "emscripten", target_os = "hurd", target_os = "redox", target_os = "dragonfly"))] {
        use libc::__errno_location as errno_location;
    } else if #[cfg(any(target_os = "solaris", target_os = "illumos"))] {
        use libc::___errno as errno_location;
    } else if #[cfg(any(target_os = "macos", target_os = "freebsd"))] {
        use libc::__error as errno_location;
    } else if #[cfg(target_os = "haiku")] {
        use libc::_errnop as errno_location;
    } else if #[cfg(target_os = "nto")] {
        use libc::__get_errno_ptr as errno_location;
    } else if #[cfg(any(all(target_os = "horizon", target_arch = "arm"), target_os = "vita"))] {
        extern "C" {
            // Not provided by libc: https://github.com/rust-lang/libc/issues/1995
            fn __errno() -> *mut libc::c_int;
        }
        use __errno as errno_location;
    } else if #[cfg(target_os = "aix")] {
        use libc::_Errno as errno_location;
    }
}

cfg_if! {
    if #[cfg(target_os = "vxworks")] {
        use libc::errnoGet as get_errno;
    } else {
        unsafe fn get_errno() -> libc::c_int { *errno_location() }
    }
}

pub(crate) fn last_os_error() -> Error {
    let errno: libc::c_int = unsafe { get_errno() };

    // c_int-to-u32 conversion is lossless for nonnegative values if they are the same size.
    const _: () = assert!(core::mem::size_of::<libc::c_int>() == core::mem::size_of::<u32>());

    match u32::try_from(errno) {
        Ok(code) if code != 0 => Error::from_os_error(code),
        _ => Error::ERRNO_NOT_POSITIVE,
    }
}

/// Fill a buffer by repeatedly invoking `sys_fill`.
///
/// The `sys_fill` function:
///   - should return -1 and set errno on failure
///   - should return the number of bytes written on success
#[allow(dead_code)]
pub(crate) fn sys_fill_exact(
    mut buf: &mut [MaybeUninit<u8>],
    sys_fill: impl Fn(&mut [MaybeUninit<u8>]) -> libc::ssize_t,
) -> Result<(), Error> {
    while !buf.is_empty() {
        let res = sys_fill(buf);
        match res {
            res if res > 0 => {
                let len = usize::try_from(res).map_err(|_| Error::UNEXPECTED)?;
                buf = buf.get_mut(len..).ok_or(Error::UNEXPECTED)?;
            }
            -1 => {
                let err = last_os_error();
                // We should try again if the call was interrupted.
                if err.raw_os_error() != Some(libc::EINTR) {
                    return Err(err);
                }
            }
            // Negative return codes not equal to -1 should be impossible.
            // EOF (ret = 0) should be impossible, as the data we are reading
            // should be an infinite stream of random bytes.
            _ => return Err(Error::UNEXPECTED),
        }
    }
    Ok(())
}
