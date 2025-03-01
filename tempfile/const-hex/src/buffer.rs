use crate::{byte2hex, imp};
use core::fmt;
use core::slice;
use core::str;

#[cfg(feature = "alloc")]
#[allow(unused_imports)]
use alloc::{string::String, vec::Vec};

/// A correctly sized stack allocation for the formatted bytes to be written
/// into.
///
/// `N` is the amount of bytes of the input, while `PREFIX` specifies whether
/// the "0x" prefix is prepended to the output.
///
/// Note that this buffer will contain only the prefix, if specified, and null
/// ('\0') bytes before any formatting is done.
///
/// # Examples
///
/// ```
/// let mut buffer = const_hex::Buffer::<4>::new();
/// let printed = buffer.format(b"1234");
/// assert_eq!(printed, "31323334");
/// ```
#[must_use]
#[repr(C)]
#[derive(Clone)]
pub struct Buffer<const N: usize, const PREFIX: bool = false> {
    // Workaround for Rust issue #76560:
    // https://github.com/rust-lang/rust/issues/76560
    // This would ideally be `[u8; (N + PREFIX as usize) * 2]`
    prefix: [u8; 2],
    bytes: [[u8; 2]; N],
}

impl<const N: usize, const PREFIX: bool> Default for Buffer<N, PREFIX> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize, const PREFIX: bool> fmt::Debug for Buffer<N, PREFIX> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Buffer").field(&self.as_str()).finish()
    }
}

impl<const N: usize, const PREFIX: bool> Buffer<N, PREFIX> {
    /// The length of the buffer in bytes.
    pub const LEN: usize = (N + PREFIX as usize) * 2;

    const ASSERT_SIZE: () = assert!(core::mem::size_of::<Self>() == 2 + N * 2, "invalid size");
    const ASSERT_ALIGNMENT: () = assert!(core::mem::align_of::<Self>() == 1, "invalid alignment");

    /// This is a cheap operation; you don't need to worry about reusing buffers
    /// for efficiency.
    #[inline]
    pub const fn new() -> Self {
        let () = Self::ASSERT_SIZE;
        let () = Self::ASSERT_ALIGNMENT;
        Self {
            prefix: if PREFIX { [b'0', b'x'] } else { [0, 0] },
            bytes: [[0; 2]; N],
        }
    }

    /// Print an array of bytes into this buffer.
    #[inline]
    pub const fn const_format(self, array: &[u8; N]) -> Self {
        self.const_format_inner::<false>(array)
    }

    /// Print an array of bytes into this buffer.
    #[inline]
    pub const fn const_format_upper(self, array: &[u8; N]) -> Self {
        self.const_format_inner::<true>(array)
    }

    /// Same as `encode_to_slice_inner`, but const-stable.
    const fn const_format_inner<const UPPER: bool>(mut self, array: &[u8; N]) -> Self {
        let mut i = 0;
        while i < N {
            let (high, low) = byte2hex::<UPPER>(array[i]);
            self.bytes[i][0] = high;
            self.bytes[i][1] = low;
            i += 1;
        }
        self
    }

    /// Print an array of bytes into this buffer and return a reference to its
    /// *lower* hex string representation within the buffer.
    #[inline]
    pub fn format(&mut self, array: &[u8; N]) -> &mut str {
        // length of array is guaranteed to be N.
        self.format_inner::<false>(array)
    }

    /// Print an array of bytes into this buffer and return a reference to its
    /// *upper* hex string representation within the buffer.
    #[inline]
    pub fn format_upper(&mut self, array: &[u8; N]) -> &mut str {
        // length of array is guaranteed to be N.
        self.format_inner::<true>(array)
    }

    /// Print a slice of bytes into this buffer and return a reference to its
    /// *lower* hex string representation within the buffer.
    ///
    /// # Panics
    ///
    /// If the slice is not exactly `N` bytes long.
    #[track_caller]
    #[inline]
    pub fn format_slice<T: AsRef<[u8]>>(&mut self, slice: T) -> &mut str {
        self.format_slice_inner::<false>(slice.as_ref())
    }

    /// Print a slice of bytes into this buffer and return a reference to its
    /// *upper* hex string representation within the buffer.
    ///
    /// # Panics
    ///
    /// If the slice is not exactly `N` bytes long.
    #[track_caller]
    #[inline]
    pub fn format_slice_upper<T: AsRef<[u8]>>(&mut self, slice: T) -> &mut str {
        self.format_slice_inner::<true>(slice.as_ref())
    }

    // Checks length
    #[track_caller]
    fn format_slice_inner<const UPPER: bool>(&mut self, slice: &[u8]) -> &mut str {
        assert_eq!(slice.len(), N, "length mismatch");
        self.format_inner::<UPPER>(slice)
    }

    // Doesn't check length
    #[inline]
    fn format_inner<const UPPER: bool>(&mut self, input: &[u8]) -> &mut str {
        // SAFETY: Length was checked previously;
        // we only write only ASCII bytes.
        unsafe {
            let buf = self.as_mut_bytes();
            let output = buf.as_mut_ptr().add(PREFIX as usize * 2);
            imp::encode::<UPPER>(input, output);
            str::from_utf8_unchecked_mut(buf)
        }
    }

    /// Copies `self` into a new owned `String`.
    #[cfg(feature = "alloc")]
    #[inline]
    #[allow(clippy::inherent_to_string)] // this is intentional
    pub fn to_string(&self) -> String {
        // SAFETY: The buffer always contains valid UTF-8.
        unsafe { String::from_utf8_unchecked(self.as_bytes().to_vec()) }
    }

    /// Returns a reference to the underlying bytes casted to a string slice.
    #[inline]
    pub const fn as_str(&self) -> &str {
        // SAFETY: The buffer always contains valid UTF-8.
        unsafe { str::from_utf8_unchecked(self.as_bytes()) }
    }

    /// Returns a mutable reference to the underlying bytes casted to a string
    /// slice.
    #[inline]
    pub fn as_mut_str(&mut self) -> &mut str {
        // SAFETY: The buffer always contains valid UTF-8.
        unsafe { str::from_utf8_unchecked_mut(self.as_mut_bytes()) }
    }

    /// Copies `self` into a new `Vec`.
    #[cfg(feature = "alloc")]
    #[inline]
    pub fn to_vec(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }

    /// Returns a reference the underlying stack-allocated byte array.
    ///
    /// # Panics
    ///
    /// If `LEN` does not equal `Self::LEN`.
    ///
    /// This is panic is evaluated at compile-time if the `nightly` feature
    /// is enabled, as inline `const` blocks are currently unstable.
    ///
    /// See Rust tracking issue [#76001](https://github.com/rust-lang/rust/issues/76001).
    #[inline]
    pub const fn as_byte_array<const LEN: usize>(&self) -> &[u8; LEN] {
        maybe_const_assert!(LEN == Self::LEN, "`LEN` must be equal to `Self::LEN`");
        // SAFETY: [u16; N] is layout-compatible with [u8; N * 2].
        unsafe { &*self.as_ptr().cast::<[u8; LEN]>() }
    }

    /// Returns a mutable reference the underlying stack-allocated byte array.
    ///
    /// # Panics
    ///
    /// If `LEN` does not equal `Self::LEN`.
    ///
    /// See [`as_byte_array`](Buffer::as_byte_array) for more information.
    #[inline]
    pub fn as_mut_byte_array<const LEN: usize>(&mut self) -> &mut [u8; LEN] {
        maybe_const_assert!(LEN == Self::LEN, "`LEN` must be equal to `Self::LEN`");
        // SAFETY: [u16; N] is layout-compatible with [u8; N * 2].
        unsafe { &mut *self.as_mut_ptr().cast::<[u8; LEN]>() }
    }

    /// Returns a reference to the underlying bytes.
    #[inline]
    pub const fn as_bytes(&self) -> &[u8] {
        // SAFETY: [u16; N] is layout-compatible with [u8; N * 2].
        unsafe { slice::from_raw_parts(self.as_ptr(), Self::LEN) }
    }

    /// Returns a mutable reference to the underlying bytes.
    ///
    /// # Safety
    ///
    /// The caller must ensure that the content of the slice is valid UTF-8
    /// before the borrow ends and the underlying `str` is used.
    ///
    /// Use of a `str` whose contents are not valid UTF-8 is undefined behavior.
    #[inline]
    pub unsafe fn as_mut_bytes(&mut self) -> &mut [u8] {
        // SAFETY: [u16; N] is layout-compatible with [u8; N * 2].
        unsafe { slice::from_raw_parts_mut(self.as_mut_ptr(), Self::LEN) }
    }

    /// Returns a mutable reference to the underlying buffer, excluding the prefix.
    ///
    /// # Safety
    ///
    /// See [`as_mut_bytes`](Buffer::as_mut_bytes).
    #[inline]
    pub unsafe fn buffer(&mut self) -> &mut [u8] {
        unsafe { slice::from_raw_parts_mut(self.bytes.as_mut_ptr().cast(), N * 2) }
    }

    /// Returns a raw pointer to the buffer.
    ///
    /// The caller must ensure that the buffer outlives the pointer this
    /// function returns, or else it will end up pointing to garbage.
    #[inline]
    pub const fn as_ptr(&self) -> *const u8 {
        unsafe { (self as *const Self).cast::<u8>().add(!PREFIX as usize * 2) }
    }

    /// Returns an unsafe mutable pointer to the slice's buffer.
    ///
    /// The caller must ensure that the slice outlives the pointer this
    /// function returns, or else it will end up pointing to garbage.
    #[inline]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        unsafe { (self as *mut Self).cast::<u8>().add(!PREFIX as usize * 2) }
    }
}
