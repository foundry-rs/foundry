use core::{mem, ptr, marker, slice, fmt, cmp};

use error_code::ErrorCode;

use crate::{sys, SysResult};
use crate::types::{c_void, c_uint};

const GHND: c_uint = 0x42;

const BYTES_LAYOUT: alloc::alloc::Layout = alloc::alloc::Layout::new::<u8>();

#[cold]
#[inline(never)]
pub fn unlikely_empty_size_result<T: Default>() -> T {
    Default::default()
}

#[cold]
#[inline(never)]
pub fn unlikely_last_error() -> ErrorCode {
    ErrorCode::last_system()
}

#[inline]
fn noop(_: *mut c_void) {
}

#[inline]
fn free_rust_mem(data: *mut c_void) {
    unsafe {
        alloc::alloc::dealloc(data as _, BYTES_LAYOUT)
    }
}

#[inline]
fn unlock_data(data: *mut c_void) {
    unsafe {
        sys::GlobalUnlock(data);
    }
}

#[inline]
fn free_global_mem(data: *mut c_void) {
    unsafe {
        sys::GlobalFree(data);
    }
}

pub struct Scope<T: Copy>(pub T, pub fn(T));

impl<T: Copy> Drop for Scope<T> {
    #[inline(always)]
    fn drop(&mut self) {
        (self.1)(self.0)
    }
}

pub struct RawMem(Scope<*mut c_void>);

impl RawMem {
    #[inline(always)]
    pub fn new_rust_mem(size: usize) -> SysResult<Self> {
        let mem = unsafe {
            alloc::alloc::alloc_zeroed(alloc::alloc::Layout::array::<u8>(size).expect("To create layout for bytes"))
        };

        if mem.is_null() {
            Err(unlikely_last_error())
        } else {
            Ok(Self(Scope(mem as _, free_rust_mem)))
        }
    }

    #[inline(always)]
    pub fn new_global_mem(size: usize) -> SysResult<Self> {
        unsafe {
            let mem = sys::GlobalAlloc(GHND, size as _);
            if mem.is_null() {
                Err(unlikely_last_error())
            } else {
                Ok(Self(Scope(mem, free_global_mem)))
            }
        }
    }

    #[inline(always)]
    pub fn from_borrowed(ptr: ptr::NonNull<c_void>) -> Self {
        Self(Scope(ptr.as_ptr(), noop))
    }

    #[inline(always)]
    pub fn get(&self) -> *mut c_void {
        (self.0).0
    }

    #[inline(always)]
    pub fn release(self) {
        mem::forget(self)
    }

    pub fn lock(&self) -> SysResult<(ptr::NonNull<c_void>, Scope<*mut c_void>)> {
        let ptr = unsafe {
            sys::GlobalLock(self.get())
        };

        match ptr::NonNull::new(ptr) {
            Some(ptr) => Ok((ptr, Scope(self.get(), unlock_data))),
            None => Err(ErrorCode::last_system()),
        }
    }
}

pub struct Buffer<'a> {
    ptr: *mut u8,
    len: usize,
    capacity: usize,
    _lifetime: marker::PhantomData<&'a str>
}

impl<'a> Buffer<'a> {
    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.capacity.saturating_sub(self.len)
    }

    #[inline]
    fn push_data(&mut self, text: &[u8]) -> usize {
        let mut write_len = cmp::min(self.remaining(), text.len());

        #[inline(always)]
        fn is_char_boundary(text: &[u8], idx: usize) -> bool {
            if idx == 0 {
                return true;
            }

            match text.get(idx) {
                None => idx == text.len(),
                Some(&byte) => (byte as i8) >= -0x40
            }
        }

        #[inline(never)]
        #[cold]
        fn shift_by_char_boundary(text: &[u8], mut size: usize) -> usize {
            while !is_char_boundary(text, size) {
                size -= 1;
            }
            size
        }

        if !is_char_boundary(text, write_len) {
            //0 is always char boundary so 0 - 1 is impossible
            write_len = shift_by_char_boundary(text, write_len - 1);
        }

        unsafe {
            ptr::copy_nonoverlapping(text.as_ptr(), self.ptr.add(self.len), write_len);
        }
        self.len += write_len;
        write_len
    }

    #[inline(always)]
    pub fn push_str(&mut self, input: &str) -> usize {
        self.push_data(input.as_bytes())
    }

    #[inline(always)]
    pub fn as_slice(&self) -> &'a [u8] {
        unsafe {
            slice::from_raw_parts(self.ptr, self.len)
        }
    }

    #[inline(always)]
    pub fn as_str(&self) -> Option<&'a str> {
        core::str::from_utf8(self.as_slice()).ok()
    }

    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.ptr
    }

    pub unsafe fn set_len(&mut self, len: usize) {
        debug_assert!(len <= self.capacity);
        self.len = len;
    }
}

impl<'a> From<&'a mut [u8]> for Buffer<'a> {
    #[inline(always)]
    fn from(this: &'a mut [u8]) -> Self {
        Self {
            ptr: this.as_mut_ptr(),
            len: 0,
            capacity: this.len(),
            _lifetime: marker::PhantomData,
        }
    }
}

impl<'a> From<&'a mut [mem::MaybeUninit<u8>]> for Buffer<'a> {
    #[inline(always)]
    fn from(this: &'a mut [mem::MaybeUninit<u8>]) -> Self {
        Self {
            ptr: this.as_mut_ptr() as *mut u8,
            len: 0,
            capacity: this.len(),
            _lifetime: marker::PhantomData,
        }
    }
}

impl<'a> fmt::Write for Buffer<'a> {
    #[inline(always)]
    fn write_str(&mut self, input: &str) -> fmt::Result {
        let result = self.push_str(input);
        if result == input.len() {
            Ok(())
        } else {
            Err(fmt::Error)
        }
    }
}
