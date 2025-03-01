//! Error code library provides generic errno/winapi error wrapper
//!
//! User can define own [Category](struct.Category.html) if you want to create new error wrapper.
//!
//! ## Usage
//!
//! ```rust
//! use error_code::ErrorCode;
//!
//! use std::fs::File;
//!
//! File::open("non_existing");
//! println!("{}", ErrorCode::last_system());
//! ```

#![no_std]
#![warn(missing_docs)]
#![cfg_attr(feature = "cargo-clippy", allow(clippy::style))]

#[cfg(feature = "std")]
extern crate std;

use core::{mem, hash, fmt};

#[deprecated]
///Text to return when cannot map error
pub const UNKNOWN_ERROR: &str = "Unknown error";
///Text to return when error fails to be converted into utf-8
pub const FAIL_ERROR_FORMAT: &str = "Failed to format error into utf-8";

///Error message buffer size
pub const MESSAGE_BUF_SIZE: usize = 256;
///Type alias for buffer to hold error code description.
pub type MessageBuf = [mem::MaybeUninit<u8>; MESSAGE_BUF_SIZE];

pub mod defs;
pub mod types;
pub mod utils;
mod posix;
pub use posix::POSIX_CATEGORY;
mod system;
pub use system::SYSTEM_CATEGORY;

#[macro_export]
///Defines error code `Category` as enum which implements conversion into generic ErrorCode
///
///This enum shall implement following traits:
///
///- `Clone`
///- `Copy`
///- `Debug`
///- `Display` - uses `ErrorCode` `fmt::Display`
///- `PartialEq` / `Eq`
///- `PartialOrd` / `Ord`
///
///# Usage
///
///```
///use error_code::{define_category, ErrorCode};
///
///define_category!(
///    ///This is documentation for my error
///    ///
///    ///Documentation of variants only allow 1 line comment and it should be within 256 characters
///    pub enum MyError {
///        ///Success
///        Success = 0,
///        ///This is bad
///        Error = 1,
///    }
///);
///
///fn handle_error(res: Result<(), MyError>) -> Result<(), ErrorCode> {
///    res?;
///    Ok(())
///}
///
///let error = handle_error(Err(MyError::Error)).expect_err("Should return error");
///assert_eq!(error.to_string(), "MyError(1): This is bad");
///assert_eq!(error.to_string(), MyError::Error.to_string());
///```
macro_rules! define_category {
    (
        $(#[$docs:meta])*
        pub enum $name:ident {
            $(
                #[doc = $msg:literal]
                $ident:ident = $code:literal,
             )+
        }
    ) => {
        #[derive(Copy, Clone, PartialEq, Eq, Debug, PartialOrd, Ord)]
        #[repr(i32)]
        $(#[$docs])*
        pub enum $name {
            $(
                #[doc = $msg]
                $ident = $code,
            )+
        }

        impl From<$name> for $crate::ErrorCode {
            #[inline(always)]
            fn from(this: $name) -> $crate::ErrorCode {
                this.into_error_code()
            }
        }

        impl core::fmt::Display for $name {
            #[inline(always)]
            fn fmt(&self, fmt: &mut core::fmt::Formatter) -> core::fmt::Result {
                core::fmt::Display::fmt(&self.into_error_code(), fmt)
            }
        }

        impl $name {
            const _ASSERT: () = {
                $(
                    assert!($msg.len() <= $crate::MESSAGE_BUF_SIZE, "Message buffer overflow, make sure your messages are not beyond MESSAGE_BUF_SIZE");
                )+
            };


            #[inline(always)]
            ///Map raw error code to textual representation.
            pub fn map_code(code: $crate::types::c_int) -> Option<&'static str> {
                match code {
                    $($code => Some($msg),)+
                    _ => None,
                }
            }

            fn message(code: $crate::types::c_int, out: &mut $crate::MessageBuf) -> &str {
                let msg = match Self::map_code(code) {
                    Some(msg) => msg,
                    None => $crate::utils::generic_map_error_code(code),
                };

                debug_assert!(msg.len() <= out.len());
                unsafe {
                    core::ptr::copy_nonoverlapping(msg.as_ptr(), out.as_mut_ptr() as *mut u8, msg.len());
                    core::str::from_utf8_unchecked(
                        core::slice::from_raw_parts(out.as_ptr() as *const u8, msg.len())
                    )
                }
            }

            ///Converts into error code
            pub fn into_error_code(self) -> $crate::ErrorCode {
                let _ = Self::_ASSERT;

                static CATEGORY: $crate::Category = $crate::Category {
                    name: core::stringify!($name),
                    message: $name::message,
                    equivalent,
                    is_would_block
                };

                fn equivalent(code: $crate::types::c_int, other: &$crate::ErrorCode) -> bool {
                    core::ptr::eq(&CATEGORY, other.category()) && code == other.raw_code()
                }

                fn is_would_block(_: $crate::types::c_int) -> bool {
                    false
                }

                $crate::ErrorCode::new(self as _, &CATEGORY)
            }
        }
    }
}

///Interface for error category
///
///It is implemented as pointers in order to avoid generics or overhead of fat pointers.
///
///## Custom implementation example
///
///```rust
///use error_code::{ErrorCode, Category};
///use error_code::types::c_int;
///
///use core::ptr;
///
///static MY_CATEGORY: Category = Category {
///    name: "MyError",
///    message,
///    equivalent,
///    is_would_block
///};
///
///fn equivalent(code: c_int, other: &ErrorCode) -> bool {
///    ptr::eq(&MY_CATEGORY, other.category()) && code == other.raw_code()
///}
///
///fn is_would_block(_: c_int) -> bool {
///    false
///}
///
///fn message(code: c_int, out: &mut error_code::MessageBuf) -> &str {
///    let msg = match code {
///        0 => "Success",
///        1 => "Bad",
///        _ => "Whatever",
///    };
///
///    debug_assert!(msg.len() <= out.len());
///    unsafe {
///        ptr::copy_nonoverlapping(msg.as_ptr(), out.as_mut_ptr() as *mut u8, msg.len())
///    }
///    msg
///}
///
///#[inline(always)]
///pub fn my_error(code: c_int) -> ErrorCode {
///    ErrorCode::new(code, &MY_CATEGORY)
///}
///```
pub struct Category {
    ///Category name
    pub name: &'static str,
    ///Maps error code and writes descriptive error message accordingly.
    ///
    ///In case of insufficient buffer, prefer to truncate message or just don't write big ass message.
    ///
    ///In case of error, just write generic name.
    ///
    ///Returns formatted message as string.
    pub message: fn(types::c_int, &mut MessageBuf) -> &str,
    ///Checks whether error code is equivalent to another one.
    ///
    ///## Args:
    ///
    ///- Raw error code, belonging to this category
    ///- Another error code being compared against this category.
    ///
    ///## Recommendation
    ///
    ///Generally error code is equal if it belongs to the same category (use `ptr::eq` to compare
    ///pointers to `Category`) and raw error codes are equal.
    pub equivalent: fn(types::c_int, &ErrorCode) -> bool,
    ///Returns `true` if supplied error code indicates WouldBlock like error.
    ///
    ///This should `true` only for errors that indicate operation can be re-tried later.
    pub is_would_block: fn(types::c_int) -> bool,
}

#[derive(Copy, Clone)]
///Describes error code of particular category.
pub struct ErrorCode {
    code: types::c_int,
    category: &'static Category
}

impl ErrorCode {
    #[inline]
    ///Initializes error code with provided category
    pub const fn new(code: types::c_int, category: &'static Category) -> Self {
        Self {
            code,
            category,
        }
    }

    #[inline(always)]
    ///Creates new POSIX error code.
    pub fn new_posix(code: types::c_int) -> Self {
        Self::new(code, &POSIX_CATEGORY)
    }

    #[inline(always)]
    ///Creates new System error code.
    pub fn new_system(code: types::c_int) -> Self {
        Self::new(code, &SYSTEM_CATEGORY)
    }

    #[inline]
    ///Gets last POSIX error
    pub fn last_posix() -> Self {
        Self::new_posix(posix::get_last_error())
    }

    #[inline]
    ///Gets last System error
    pub fn last_system() -> Self {
        Self::new_system(system::get_last_error())
    }

    #[inline(always)]
    ///Gets raw error code.
    pub const fn raw_code(&self) -> types::c_int {
        self.code
    }

    #[inline(always)]
    ///Gets reference to underlying Category.
    pub const fn category(&self) -> &'static Category {
        self.category
    }

    #[inline(always)]
    ///Returns `true` if underlying error indicates operation can or should be re-tried at later date.
    pub fn is_would_block(&self) -> bool {
        (self.category.is_would_block)(self.code)
    }
}

impl PartialEq for ErrorCode {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        (self.category.equivalent)(self.code, other)
    }
}

impl Eq for ErrorCode {}

impl hash::Hash for ErrorCode {
    #[inline]
    fn hash<H: hash::Hasher>(&self, state: &mut H) {
        self.code.hash(state);
    }
}

impl fmt::Debug for ErrorCode {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = [mem::MaybeUninit::uninit(); MESSAGE_BUF_SIZE];
        let message = (self.category.message)(self.code, &mut out);
        fmt.debug_struct(self.category.name).field("code", &self.code).field("message", &message).finish()
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = [mem::MaybeUninit::uninit(); MESSAGE_BUF_SIZE];
        let message = (self.category.message)(self.code, &mut out);
        fmt.write_fmt(format_args!("{}({}): {}", self.category.name, self.code, message))
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ErrorCode {}

#[cfg(feature = "std")]
impl From<std::io::Error> for ErrorCode {
    #[inline]
    fn from(err: std::io::Error) -> Self {
        match err.raw_os_error() {
            Some(err) => Self::new_posix(err),
            None => Self::new_posix(-1),
        }
    }
}
