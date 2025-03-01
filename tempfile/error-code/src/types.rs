//! C types used inside crate
#![allow(non_camel_case_types)]

//https://github.com/rust-lang/rust/blob/7b4d9e155fec06583c763f176fc432dc779f1fc6/library/core/src/ffi/mod.rs#L166
#[cfg(any(target_arch = "avr", target_arch = "msp430"))]
mod ints {
    ///C type `int`
    pub type c_int = i16;
    ///C type `unsigned int`
    pub type c_uint = u16;
}

#[cfg(not(any(target_arch = "avr", target_arch = "msp430")))]
mod ints {
    ///C type `int`
    pub type c_int = i32;
    ///C type `unsigned int`
    pub type c_uint = u32;
}

#[cfg(all(target_pointer_width = "64", not(windows)))]
mod longs {
    ///C type `unsigned long`
    pub type c_ulong = u64;
}
#[cfg(not(all(target_pointer_width = "64", not(windows))))]
mod longs {
    ///C type `unsigned long`
    pub type c_ulong = u32;
}

pub use ints::*;
pub use longs::*;
