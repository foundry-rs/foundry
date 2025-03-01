//! Definitions used by this library

#[allow(unused)]
use crate::types::c_int;

#[cfg(any(target_env = "wasi", target_os = "wasi"))]
///EAGAIN
pub const EAGAIN: c_int = 6;
#[cfg(any(target_env = "wasi", target_os = "wasi"))]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

#[cfg(windows)]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(windows)]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = 140;

#[cfg(target_os = "fuchsia")]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(target_os = "fuchsia")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

#[cfg(target_os = "solid_asp3")]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(target_os = "solid_asp3")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

#[cfg(target_os = "vxworks")]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(target_os = "vxworks")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = 70;

#[cfg(target_os = "teeos")]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(target_os = "teeos")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

//---------------------
//unix
//---------------------
#[cfg(
    any(
        target_env = "newlib",
        target_os = "solaris", target_os = "illumos", target_os = "nto",
        target_os = "aix", target_os = "android", target_os = "linux",
        target_os = "l4re"
    )
)]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(
    any(
        target_os = "solaris", target_os = "illumos", target_os = "nto",
        target_os = "aix", target_os = "android", target_os = "linux",
        target_os = "l4re"
    )
)]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;
#[cfg(
    any(
        target_os = "macos", target_os = "ios", target_os = "tvos",
        target_os = "watchos", target_os = "freebsd", target_os = "dragonfly",
        target_os = "openbsd", target_os = "netbsd"
    )
)]
///EAGAIN
pub const EAGAIN: c_int = 35;
#[cfg(
    any(
        target_os = "macos", target_os = "ios", target_os = "tvos",
        target_os = "watchos", target_os = "freebsd", target_os = "dragonfly",
        target_os = "openbsd", target_os = "netbsd"
    )
)]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

#[cfg(target_os = "redox")]
///EAGAIN
pub const EAGAIN: c_int = 11;
#[cfg(target_os = "redox")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = 41;

#[cfg(target_os = "haiku")]
///EAGAIN
pub const EAGAIN: c_int = -2147483637;
#[cfg(target_os = "haiku")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;

#[cfg(target_os = "emscripten")]
///EAGAIN
pub const EAGAIN: c_int = 6;
#[cfg(target_os = "emscripten")]
///EWOULDBLOCK
pub const EWOULDBLOCK: c_int = EAGAIN;
