//!Raw bindings to Windows clipboard.
//!
//!## General information
//!
//!All pre & post conditions are stated in description of functions.
//!
//!### Open clipboard
//! To access any information inside clipboard it is necessary to open it by means of
//! [open()](fn.open.html).
//!
//! After that Clipboard cannot be opened any more until [close()](fn.close.html) is called.

use crate::types::*;
use crate::sys::*;
use crate::utils::Buffer;
use crate::options::{self, EmptyFn, Clearing};

const CBM_INIT: DWORD = 0x04;
const BI_RGB: DWORD = 0;
const DIB_RGB_COLORS: DWORD = 0;
const ERROR_INCORRECT_SIZE: DWORD = 1462;
const CP_UTF8: DWORD = 65001;

use error_code::ErrorCode;

use core::{slice, mem, ptr, cmp, str, hint};
use core::num::{NonZeroUsize, NonZeroU32};

use alloc::string::String;
use alloc::borrow::ToOwned;
use alloc::format;

use crate::{SysResult, html, formats};
use crate::utils::{unlikely_empty_size_result, RawMem};

#[cold]
#[inline(never)]
fn invalid_data() -> ErrorCode {
    ErrorCode::new_system(13)
}

#[inline(always)]
fn free_dc(data: HDC) {
    unsafe {
        ReleaseDC(ptr::null_mut(), data);
    }
}

/// A scope guard for impersonating anonymous token on Windows
struct AnonymousTokenImpersonator {
    must_revert: bool,
}

impl AnonymousTokenImpersonator {
    #[inline]
    pub fn new() -> Self {
        Self {
            must_revert: unsafe { ImpersonateAnonymousToken(GetCurrentThread()) != 0 },
        }
    }
}

impl Drop for AnonymousTokenImpersonator {
    #[inline]
    fn drop(&mut self) {
        if self.must_revert {
            unsafe {
                RevertToSelf();
            }
        }
    }
}

#[inline(always)]
///Opens clipboard.
///
///Wrapper around ```OpenClipboard```.
///
///# Pre-conditions:
///
///* Clipboard is not opened yet.
///
///# Post-conditions (if successful):
///
///* Clipboard can be accessed for read and write operations.
pub fn open() -> SysResult<()> {
    open_for(ptr::null_mut())
}

#[allow(clippy::not_unsafe_ptr_arg_deref)]
//clippy is retard
#[inline]
///Opens clipboard associating it with specified window handle.
///
///Unless [empty](fn.empty.html) is called, `owner` would be `None`.
///
///Wrapper around ```OpenClipboard```.
///
///# Pre-conditions:
///
///* Clipboard is not opened yet.
///
///# Post-conditions (if successful):
///
///* Clipboard can be accessed for read and write operations.
pub fn open_for(owner: HWND) -> SysResult<()> {
    match unsafe { OpenClipboard(owner) } {
        0 => Err(ErrorCode::last_system()),
        _ => Ok(()),
    }
}

#[inline]
///Closes clipboard.
///
///Wrapper around ```CloseClipboard```.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
pub fn close() -> SysResult<()> {
    // See https://crbug.com/441834:
    //  Impersonate anonymous token while calling CloseClipboard.
    //  This prevents the Windows kernel from capturing the broker's
    //  access token which could lead to potential escalation of privilege.
    let _guard = AnonymousTokenImpersonator::new();

    match unsafe { CloseClipboard() } {
        0 => Err(ErrorCode::last_system()),
        _ => Ok(()),
    }
}

#[inline]
///Empties clipboard.
///
///Wrapper around ```EmptyClipboard```.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
pub fn empty() -> SysResult<()> {
    match unsafe { EmptyClipboard() } {
        0 => Err(ErrorCode::last_system()),
        _ => Ok(()),
    }
}

#[inline]
///Retrieves clipboard sequence number.
///
///Wrapper around ```GetClipboardSequenceNumber```.
///
///# Returns:
///
///* ```Some``` Contains return value of ```GetClipboardSequenceNumber```.
///* ```None``` In case if you do not have access. It means that zero is returned by system.
pub fn seq_num() -> Option<NonZeroU32> {
    unsafe { NonZeroU32::new(GetClipboardSequenceNumber()) }
}

#[inline]
///Retrieves size of clipboard data for specified format.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
///
///# Returns:
///
///Size in bytes if format presents on clipboard.
///
///# Unsafety:
///
///In some cases, clipboard content might be so invalid that it crashes on `GlobalSize` (e.g.
///Bitmap)
///
///Due to that function is marked as unsafe
pub unsafe fn size_unsafe(format: u32) -> Option<NonZeroUsize> {
    let clipboard_data = GetClipboardData(format);

    match clipboard_data.is_null() {
        false => NonZeroUsize::new(GlobalSize(clipboard_data) as usize),
        true => None,
    }
}

#[inline]
///Retrieves size of clipboard data for specified format.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
///
///# Returns:
///
///Size in bytes if format is presents on clipboard.
pub fn size(format: u32) -> Option<NonZeroUsize> {
    let clipboard_data = unsafe {GetClipboardData(format)};

    if clipboard_data.is_null() {
        return None
    }

    unsafe {
        if GlobalLock(clipboard_data).is_null() {
            return None;
        }

        let result = NonZeroUsize::new(GlobalSize(clipboard_data) as usize);

        GlobalUnlock(clipboard_data);

        result
    }
}

#[inline(always)]
///Retrieves raw pointer to clipboard data.
///
///Wrapper around ```GetClipboardData```.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
pub fn get_clipboard_data(format: c_uint) -> SysResult<ptr::NonNull<c_void>> {
    let ptr = unsafe {
        GetClipboardData(format)
    };
    match ptr::NonNull::new(ptr) {
        Some(ptr) => Ok(ptr),
        None => Err(ErrorCode::last_system()),
    }
}

#[inline(always)]
///Determines whenever provided clipboard format is available on clipboard or not.
pub fn is_format_avail(format: c_uint) -> bool {
    unsafe { IsClipboardFormatAvailable(format) != 0 }
}

#[inline(always)]
///Returns the first available format in the specified list.
///
///Returns `None` if no format is available or clipboard is empty
pub fn which_format_avail(formats: &[c_uint]) -> Option<NonZeroU32> {
    let result = unsafe {
        GetPriorityClipboardFormat(formats.as_ptr(), formats.len() as _)
    };
    if result < 0 {
        None
    } else {
        NonZeroU32::new(result as _)
    }
}


#[inline]
///Retrieves number of currently available formats on clipboard.
///
///Returns `None` if `CountClipboardFormats` failed.
pub fn count_formats() -> Option<usize> {
    let result = unsafe { CountClipboardFormats() };

    if result == 0 {
        if ErrorCode::last_system().raw_code() != 0 {
            return None
        }
    }

    Some(result as usize)
}

///Copies raw bytes from clipboard with specified `format`
///
///Returns number of copied bytes on success, otherwise 0.
///
///It is safe to pass uninit memory
pub fn get(format: u32, out: &mut [u8]) -> SysResult<usize> {
    let size = out.len();
    if size == 0 {
        return Ok(unlikely_empty_size_result());
    }
    let out_ptr = out.as_mut_ptr();

    let ptr = RawMem::from_borrowed(get_clipboard_data(format)?);

    let result = unsafe {
        let (data_ptr, _lock) = ptr.lock()?;
        let data_size = cmp::min(GlobalSize(ptr.get()) as usize, size);
        ptr::copy_nonoverlapping(data_ptr.as_ptr() as *const u8, out_ptr, data_size);
        data_size
    };

    Ok(result)
}

///Copies raw bytes from clipboard with specified `format`, appending to `out` buffer.
///
///Returns number of copied bytes on success, otherwise 0.
pub fn get_vec(format: u32, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
    let ptr = RawMem::from_borrowed(get_clipboard_data(format)?);

    let result = unsafe {
        let (data_ptr, _lock) = ptr.lock()?;
        let data_size = GlobalSize(ptr.get()) as usize;

        out.reserve(data_size as usize);
        let storage_cursor = out.len();
        let storage_ptr = out.as_mut_ptr().add(out.len()) as *mut _;

        ptr::copy_nonoverlapping(data_ptr.as_ptr() as *const u8, storage_ptr, data_size);
        out.set_len(storage_cursor + data_size as usize);

        data_size
    };

    Ok(result)
}

///Retrieves HTML using format code created by `register_raw_format` or `register_format` with argument `HTML Format`
pub fn get_html(format: u32, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
    let ptr = RawMem::from_borrowed(get_clipboard_data(format)?);

    let result = unsafe {
        let (data_ptr, _lock) = ptr.lock()?;
        let data_size = GlobalSize(ptr.get()) as usize;

        let data = match str::from_utf8(slice::from_raw_parts(data_ptr.as_ptr() as *const u8, data_size)) {
            Ok(data) => data,
            Err(_) => return Err(invalid_data()),
        };

        let mut start_idx = 0usize;
        let mut end_idx = data.len();
        for line in data.lines() {
            let mut split = line.split(html::SEP);
            let key = match split.next() {
                Some(key) => key,
                None => hint::unreachable_unchecked(),
            };
            let value = match split.next() {
                Some(value) => value,
                //Reached HTML
                None => break
            };
            match key {
                html::START_FRAGMENT => match value.trim_start_matches('0').parse() {
                    Ok(value) => {
                        start_idx = value;
                        continue;
                    }
                    //Should not really happen
                    Err(_) => break,
                },
                html::END_FRAGMENT => match value.trim_start_matches('0').parse() {
                    Ok(value) => {
                        end_idx = value;
                        continue;
                    }
                    //Should not really happen
                    Err(_) => break,
                },
                _ => continue,
            }
        }

        //Make sure HTML writer didn't screw up offsets of fragment
        let size = match end_idx.checked_sub(start_idx) {
            Some(size) => size,
            None => return Err(invalid_data()),
        };
        if size > data_size {
            return Err(invalid_data())
        }

        out.reserve(size);
        let out_cursor = out.len();
        ptr::copy_nonoverlapping(data.as_ptr().add(start_idx), out.spare_capacity_mut().as_mut_ptr().add(out_cursor) as _, size);
        out.set_len(out_cursor + size);
        size
    };

    Ok(result)
}

///Sets HTML using format code created by `register_raw_format` or `register_format` with argument `HTML Format`
///
///Allows to customize clipboard setting behavior
///
///- `C` - Specifies clearing behavior
pub fn set_html_with<C: Clearing>(format: u32, html: &str, _is_clear: C) -> SysResult<()> {
    set_html_inner(format, html, C::EMPTY_FN)
}

///Sets HTML using format code created by `register_raw_format` or `register_format` with argument `HTML Format`
pub fn set_html(format: u32, html: &str) -> SysResult<()> {
    set_html_inner(format, html, options::NoClear::EMPTY_FN)
}

fn set_html_inner(format: u32, html: &str, empty: EmptyFn) -> SysResult<()> {
    const VERSION_VALUE: &str = ":0.9";
    const HEADER_SIZE: usize = html::VERSION.len() + VERSION_VALUE.len() + html::NEWLINE.len()
                               + html::START_HTML.len() + html::LEN_SIZE + 1 + html::NEWLINE.len()
                               + html::END_HTML.len() + html::LEN_SIZE + 1 + html::NEWLINE.len()
                               + html::START_FRAGMENT.len() + html::LEN_SIZE + 1 + html::NEWLINE.len()
                               + html::END_FRAGMENT.len() + html::LEN_SIZE + 1 + html::NEWLINE.len();
    const FRAGMENT_OFFSET: usize = HEADER_SIZE + html::BODY_HEADER.len();

    let total_size = FRAGMENT_OFFSET + html::BODY_FOOTER.len() + html.len();

    let mut len_buffer = html::LengthBuffer::new();
    let mem = RawMem::new_global_mem(total_size)?;

    unsafe {
        use core::fmt::Write;
        let (ptr, _lock) = mem.lock()?;
        let out = slice::from_raw_parts_mut(ptr.as_ptr() as *mut mem::MaybeUninit<u8>, total_size);

        let mut cursor = 0;
        macro_rules! write_out {
            ($input:expr) => {
                let input = $input;
                ptr::copy_nonoverlapping(input.as_ptr() as *const u8, out.as_mut_ptr().add(cursor) as _, input.len());
                cursor += input.len();
            };
        }

        write_out!(html::VERSION);
        write_out!(VERSION_VALUE);
        write_out!(html::NEWLINE);

        let _ = write!(&mut len_buffer, "{:0>10}", HEADER_SIZE);
        write_out!(html::START_HTML);
        write_out!([html::SEP as u8]);
        write_out!(&len_buffer);
        write_out!(html::NEWLINE);

        let _ = write!(&mut len_buffer, "{:0>10}", total_size);
        write_out!(html::END_HTML);
        write_out!([html::SEP as u8]);
        write_out!(&len_buffer);
        write_out!(html::NEWLINE);

        let _ = write!(&mut len_buffer, "{:0>10}", FRAGMENT_OFFSET);
        write_out!(html::START_FRAGMENT);
        write_out!([html::SEP as u8]);
        write_out!(&len_buffer);
        write_out!(html::NEWLINE);

        let _ = write!(&mut len_buffer, "{:0>10}", total_size - html::BODY_FOOTER.len());
        write_out!(html::END_FRAGMENT);
        write_out!([html::SEP as u8]);
        write_out!(&len_buffer);
        write_out!(html::NEWLINE);

        //Verify StartHTML is correct
        debug_assert_eq!(HEADER_SIZE, cursor);

        write_out!(html::BODY_HEADER);

        //Verify StartFragment is correct
        debug_assert_eq!(FRAGMENT_OFFSET, cursor);

        write_out!(html);

        //Verify EndFragment is correct
        debug_assert_eq!(total_size - html::BODY_FOOTER.len(), cursor);

        write_out!(html::BODY_FOOTER);

        //Verify EndHTML is correct
        debug_assert_eq!(cursor, total_size);
    }

    let _ = (empty)();
    if unsafe { !SetClipboardData(format, mem.get()).is_null() } {
        //SetClipboardData takes ownership
        mem.release();
        Ok(())
    } else {
        Err(ErrorCode::last_system())
    }
}

fn set_inner(format: u32, data: &[u8], clear: EmptyFn) -> SysResult<()> {
    let size = data.len();
    if size == 0 {
        #[allow(clippy::unit_arg)]
        return Ok(unlikely_empty_size_result());
    }

    let mem = RawMem::new_global_mem(size)?;

    {
        let (ptr, _lock) = mem.lock()?;
        unsafe { ptr::copy_nonoverlapping(data.as_ptr(), ptr.as_ptr() as _, size) };
    }

    let _ = (clear)();
    if unsafe { !SetClipboardData(format, mem.get()).is_null() } {
        //SetClipboardData takes ownership
        mem.release();
        return Ok(());
    }

    Err(ErrorCode::last_system())
}
/// Copies raw bytes onto clipboard with specified `format`, returning whether it was successful.
///
/// This function empties the clipboard before setting the data.
pub fn set(format: u32, data: &[u8]) -> SysResult<()> {
    set_inner(format, data, options::DoClear::EMPTY_FN)
}

/// Copies raw bytes onto the clipboard with the specified `format`, returning whether it was successful.
///
/// This function does not empty the clipboard before setting the data.
pub fn set_without_clear(format: u32, data: &[u8]) -> SysResult<()> {
    set_inner(format, data, options::NoClear::EMPTY_FN)
}

///Copies raw bytes from clipboard with specified `format`, appending to `out` buffer.
///
///Returns number of copied bytes on success, otherwise 0.
pub fn get_string(out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
    let ptr = RawMem::from_borrowed(get_clipboard_data(formats::CF_UNICODETEXT)?);

    let result = unsafe {
        let (data_ptr, _lock) = ptr.lock()?;
        let data_size = GlobalSize(ptr.get()) as usize / mem::size_of::<u16>();
        let storage_req_size = WideCharToMultiByte(CP_UTF8, 0, data_ptr.as_ptr() as _, data_size as _, ptr::null_mut(), 0, ptr::null(), ptr::null_mut());

        if storage_req_size == 0 {
            return Err(ErrorCode::last_system());
        }

        let storage_cursor = out.len();
        out.reserve(storage_req_size as usize);
        let storage_ptr = out.as_mut_ptr().add(storage_cursor) as *mut _;
        WideCharToMultiByte(CP_UTF8, 0, data_ptr.as_ptr() as _, data_size as _, storage_ptr, storage_req_size, ptr::null(), ptr::null_mut());
        out.set_len(storage_cursor + storage_req_size as usize);

        //It seems WinAPI always supposed to have at the end null char.
        //But just to be safe let's check for it and only then remove.
        if let Some(null_idx) = out.iter().skip(storage_cursor).position(|b| *b == b'\0') {
            out.set_len(storage_cursor + null_idx);
        }

        out.len() - storage_cursor
    };

    Ok(result)
}

fn set_string_inner(data: &str, clear: EmptyFn) -> SysResult<()> {
    let size = unsafe {
        MultiByteToWideChar(CP_UTF8, 0, data.as_ptr() as *const _, data.len() as _, ptr::null_mut(), 0)
    };

    //MultiByteToWideChar fails on empty input, but we can ignore it and just set buffer with null char
    if size != 0 || data.is_empty() {
        let mem = RawMem::new_global_mem((mem::size_of::<u16>() * (size as usize + 1)) as _)?;
        {
            let (ptr, _lock) = mem.lock()?;
            let ptr = ptr.as_ptr() as *mut u16;
            unsafe {
                MultiByteToWideChar(CP_UTF8, 0, data.as_ptr() as *const _, data.len() as _, ptr, size);
                ptr::write(ptr.offset(size as isize), 0);
            }
        }

        let _ = (clear)();
        if unsafe { !SetClipboardData(formats::CF_UNICODETEXT, mem.get()).is_null() } {
            //SetClipboardData takes ownership
            mem.release();
            return Ok(());
        }
    }

    Err(ErrorCode::last_system())
}

#[inline(always)]
///Copies unicode string onto clipboard, performing necessary conversions, returning true on
///success.
pub fn set_string(data: &str) -> SysResult<()> {
    set_string_inner(data, options::DoClear::EMPTY_FN)
}

#[inline(always)]
///Copies unicode string onto clipboard, performing necessary conversions, returning true on
///success.
///
///Allows to customize clipboard setting behavior
///
///- `C` - Specifies clearing behavior
pub fn set_string_with<C: Clearing>(data: &str, _is_clear: C) -> SysResult<()> {
    set_string_inner(data, C::EMPTY_FN)
}

#[cfg(feature = "std")]
///Retrieves file list from clipboard, appending each element to the provided storage.
///
///Returns number of appended file names.
pub fn get_file_list_path(out: &mut alloc::vec::Vec<std::path::PathBuf>) -> SysResult<usize> {
    use std::os::windows::ffi::OsStringExt;

    let clipboard_data = RawMem::from_borrowed(get_clipboard_data(formats::CF_HDROP)?);

    let (_data_ptr, _lock) = clipboard_data.lock()?;

    let num_files = unsafe { DragQueryFileW(clipboard_data.get() as _, u32::MAX, ptr::null_mut(), 0) };
    out.reserve(num_files as usize);

    let mut buffer = alloc::vec::Vec::new();

    for idx in 0..num_files {
        let required_size_no_null = unsafe { DragQueryFileW(clipboard_data.get() as _, idx, ptr::null_mut(), 0) };
        if required_size_no_null == 0 {
            return Err(ErrorCode::last_system());
        }

        let required_size = required_size_no_null + 1;
        buffer.reserve(required_size as usize);

        if unsafe { DragQueryFileW(clipboard_data.get() as _, idx, buffer.as_mut_ptr(), required_size) == 0 } {
            return Err(ErrorCode::last_system());
        }

        unsafe {
            buffer.set_len(required_size_no_null as usize);
        }
        //This fucking abomination of API requires double allocation,
        //just because no one had brain for to provide API for creation OsString out of owned
        //Vec<16>
        out.push(std::ffi::OsString::from_wide(&buffer).into())
    }

    Ok(num_files as usize)
}

///Retrieves file list from clipboard, appending each element to the provided storage.
///
///Returns number of appended file names.
pub fn get_file_list(out: &mut alloc::vec::Vec<alloc::string::String>) -> SysResult<usize> {
    let clipboard_data = RawMem::from_borrowed(get_clipboard_data(formats::CF_HDROP)?);

    let (_data_ptr, _lock) = clipboard_data.lock()?;

    let num_files = unsafe { DragQueryFileW(clipboard_data.get() as _, u32::MAX, ptr::null_mut(), 0) };
    out.reserve(num_files as usize);

    let mut buffer = alloc::vec::Vec::new();

    for idx in 0..num_files {
        let required_size_no_null = unsafe { DragQueryFileW(clipboard_data.get() as _, idx, ptr::null_mut(), 0) };
        if required_size_no_null == 0 {
            return Err(ErrorCode::last_system());
        }

        let required_size = required_size_no_null + 1;
        buffer.reserve(required_size as usize);

        if unsafe { DragQueryFileW(clipboard_data.get() as _, idx, buffer.as_mut_ptr(), required_size) == 0 } {
            return Err(ErrorCode::last_system());
        }

        unsafe {
            buffer.set_len(required_size_no_null as usize);
        }
        out.push(alloc::string::String::from_utf16_lossy(&buffer));
    }

    Ok(num_files as usize)
}

///Reads bitmap image, appending image to the `out` vector and returning number of bytes read on
///success.
///
///Output will contain header following by RGB
pub fn get_bitmap(out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
    let clipboard_data = get_clipboard_data(formats::CF_BITMAP)?;

    //Thanks @matheuslessarodrigues
    let mut bitmap = BITMAP {
        bmType: 0,
        bmWidth: 0,
        bmHeight: 0,
        bmWidthBytes: 0,
        bmPlanes: 0,
        bmBitsPixel: 0,
        bmBits: ptr::null_mut(),
    };

    if unsafe { GetObjectW(clipboard_data.as_ptr(), mem::size_of::<BITMAP>() as _, &mut bitmap as *mut BITMAP as _) } == 0 {
        return Err(ErrorCode::last_system());
    }

    let clr_bits = bitmap.bmPlanes * bitmap.bmBitsPixel;
    let clr_bits = if clr_bits == 1 {
        1
    } else if clr_bits <= 4 {
        4
    } else if clr_bits <= 8 {
        8
    } else if clr_bits <= 16 {
        16
    } else if clr_bits <= 24 {
        24
    } else {
        32
    };

    let header_storage = RawMem::new_rust_mem(if clr_bits < 24 {
        mem::size_of::<BITMAPINFOHEADER>() + mem::size_of::<RGBQUAD>() * (1 << clr_bits)
    } else {
        mem::size_of::<BITMAPINFOHEADER>()
    })?;

    let header = unsafe {
        &mut *(header_storage.get() as *mut BITMAPINFO)
    };

    header.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as _;
    header.bmiHeader.biWidth = bitmap.bmWidth;
    header.bmiHeader.biHeight = bitmap.bmHeight;
    header.bmiHeader.biPlanes = bitmap.bmPlanes;
    header.bmiHeader.biBitCount = bitmap.bmBitsPixel;
    header.bmiHeader.biCompression = BI_RGB;
    if clr_bits < 24 {
        header.bmiHeader.biClrUsed = 1 << clr_bits;
    }

    header.bmiHeader.biSizeImage = ((((header.bmiHeader.biWidth * clr_bits + 31) & !31) / 8) * header.bmiHeader.biHeight) as _;
    header.bmiHeader.biClrImportant = 0;

    let img_size = header.bmiHeader.biSizeImage as usize;
    let out_before = out.len();

    let dc = crate::utils::Scope(unsafe { GetDC(ptr::null_mut()) }, free_dc);
    let mut buffer = alloc::vec![0; img_size];

    if unsafe { GetDIBits(dc.0, clipboard_data.as_ptr() as _, 0, bitmap.bmHeight as _, buffer.as_mut_ptr() as _, header_storage.get() as _, DIB_RGB_COLORS) } == 0 {
        return Err(ErrorCode::last_system());
    }

    //Write header
    out.extend_from_slice(&u16::to_le_bytes(0x4d42));
    out.extend_from_slice(&u32::to_le_bytes(mem::size_of::<BITMAPFILEHEADER>() as u32 + header.bmiHeader.biSize + header.bmiHeader.biClrUsed * mem::size_of::<RGBQUAD>() as u32 + header.bmiHeader.biSizeImage));
    out.extend_from_slice(&u32::to_le_bytes(0)); //2 * u16 of 0
    out.extend_from_slice(&u32::to_le_bytes(mem::size_of::<BITMAPFILEHEADER>() as u32 + header.bmiHeader.biSize + header.bmiHeader.biClrUsed * mem::size_of::<RGBQUAD>() as u32));

    out.extend_from_slice(&header.bmiHeader.biSize.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biWidth.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biHeight.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biPlanes.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biBitCount.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biCompression.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biSizeImage.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biXPelsPerMeter.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biYPelsPerMeter.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biClrUsed.to_le_bytes());
    out.extend_from_slice(&header.bmiHeader.biClrImportant.to_le_bytes());

    for color in unsafe { slice::from_raw_parts(header.bmiColors.as_ptr(), header.bmiHeader.biClrUsed as _) } {
        out.push(color.rgbBlue);
        out.push(color.rgbGreen);
        out.push(color.rgbRed);
        out.push(color.rgbReserved);
    }

    out.extend_from_slice(&buffer);

    Ok(out.len() - out_before)
}

#[inline(always)]
#[doc(hidden)]
pub fn set_bitamp(data: &[u8]) -> SysResult<()> {
    set_bitmap(data)
}

///Sets bitmap (header + RGB) onto clipboard, from raw bytes.
///
///Returns `ERROR_INCORRECT_SIZE` if size of data is not valid
pub fn set_bitmap(data: &[u8]) -> SysResult<()> {
    //Bitmap format cannot really overlap with much so there is no risk of having non-empty clipboard
    //Also it is backward compatible beahvior.
    //To be changed in 6.x
    set_bitmap_inner(data, options::NoClear::EMPTY_FN)
}

///Sets bitmap (header + RGB) onto clipboard, from raw bytes.
///
///Returns `ERROR_INCORRECT_SIZE` if size of data is not valid
///
///Allows to customize clipboard setting behavior
///
///- `C` - Specifies clearing behavior
pub fn set_bitmap_with<C: Clearing>(data: &[u8], _is_clear: C) -> SysResult<()> {
    set_bitmap_inner(data, C::EMPTY_FN)
}

fn set_bitmap_inner(data: &[u8], clear: EmptyFn) -> SysResult<()> {
    const FILE_HEADER_LEN: usize = mem::size_of::<BITMAPFILEHEADER>();
    const INFO_HEADER_LEN: usize = mem::size_of::<BITMAPINFOHEADER>();

    if data.len() <= (FILE_HEADER_LEN + INFO_HEADER_LEN) {
        return Err(ErrorCode::new_system(ERROR_INCORRECT_SIZE as _));
    }

    let mut file_header = mem::MaybeUninit::<BITMAPFILEHEADER>::uninit();
    let mut info_header = mem::MaybeUninit::<BITMAPINFOHEADER>::uninit();

    let (file_header, info_header) = unsafe {
        ptr::copy_nonoverlapping(data.as_ptr(), file_header.as_mut_ptr() as _, FILE_HEADER_LEN);
        ptr::copy_nonoverlapping(data.as_ptr().add(FILE_HEADER_LEN), info_header.as_mut_ptr() as _, INFO_HEADER_LEN);
        (file_header.assume_init(), info_header.assume_init())
    };

    if data.len() <= file_header.bfOffBits as usize {
        return Err(ErrorCode::new_system(ERROR_INCORRECT_SIZE as _));
    }

    let bitmap = &data[file_header.bfOffBits as _..];

    if bitmap.len() < info_header.biSizeImage as usize {
        return Err(ErrorCode::new_system(ERROR_INCORRECT_SIZE as _));
    }

    let dc = crate::utils::Scope(unsafe { GetDC(ptr::null_mut()) }, free_dc);

    let handle = unsafe {
        CreateDIBitmap(dc.0, &info_header as _, CBM_INIT, bitmap.as_ptr() as _, &info_header as *const _ as *const BITMAPINFO, DIB_RGB_COLORS)
    };

    if handle.is_null() {
        return Err(ErrorCode::last_system());
    }

    let _ = (clear)();
    if unsafe { SetClipboardData(formats::CF_BITMAP, handle as _).is_null() } {
        return Err(ErrorCode::last_system());
    }

    Ok(())
}


#[inline(always)]
///Set list of file paths to clipboard.
pub fn set_file_list(paths: &[impl AsRef<str>]) -> SysResult<()> {
    //See set_bitmap for reasoning of NoClear
    set_file_list_inner(paths, options::NoClear::EMPTY_FN)
}

#[inline(always)]
///Set list of file paths to clipboard.
pub fn set_file_list_with<C: Clearing>(paths: &[impl AsRef<str>], _is_clear: C) -> SysResult<()> {
    set_file_list_inner(paths, C::EMPTY_FN)
}

fn set_file_list_inner(paths: &[impl AsRef<str>], empty: EmptyFn) -> SysResult<()> {
    #[repr(C, packed(1))]
    pub struct DROPFILES {
        pub p_files: u32,
        pub pt: POINT,
        pub f_nc: c_int,
        pub f_wide: c_int,
    }
    const DROPFILES_SIZE: DWORD = core::mem::size_of::<DROPFILES>() as DWORD;

    let mut file_list_size = 0;
    for path in paths {
        let path = path.as_ref();
        unsafe {
            //+1 for null char
            file_list_size += MultiByteToWideChar(CP_UTF8, 0, path.as_ptr() as *const _, path.len() as _, ptr::null_mut(), 0) + 1
        }
    }

    if file_list_size == 0 {
        return Err(ErrorCode::last_system());
    }

    let dropfiles = DROPFILES {
        p_files: DROPFILES_SIZE,
        pt: POINT { x: 0, y: 0 },
        f_nc: 0,
        f_wide: 1,
    };

    let mem_size = DROPFILES_SIZE as usize + (file_list_size as usize * 2) + 2; //+2 for final null char
    let mem = crate::utils::RawMem::new_global_mem(mem_size)?;
    {
        let (ptr, _lock) = mem.lock()?;
        let ptr = ptr.as_ptr() as *mut u8;
        unsafe {
            (ptr as *mut DROPFILES).write(dropfiles);

            let mut ptr = ptr.add(DROPFILES_SIZE as usize) as *mut u16;
            for path in paths {
                let path = path.as_ref();
                let written = MultiByteToWideChar(CP_UTF8, 0, path.as_ptr() as *const _, path.len() as _, ptr, file_list_size);
                ptr = ptr.offset(written as isize);
                //Add null termination character
                ptr.write(0);
                ptr = ptr.add(1);
                file_list_size -= written - 1;
            }
            //Add final null termination, to indicate end of list
            //null-terminate string
            ptr.write(0);
        }
    }

    let _ = (empty)();
    if unsafe { !SetClipboardData(formats::CF_HDROP, mem.get()).is_null() } {
        //SetClipboardData now has ownership of `mem`.
        mem.release();
        Ok(())
    } else {
        Err(ErrorCode::last_system())
    }
}

///Enumerator over available clipboard formats.
///
///# Pre-conditions:
///
///* [open()](fn.open.html) has been called.
pub struct EnumFormats {
    idx: u32
}

impl EnumFormats {
    /// Constructs enumerator over all available formats.
    pub fn new() -> EnumFormats {
        EnumFormats { idx: 0 }
    }

    /// Constructs enumerator that starts from format.
    pub fn from(format: u32) -> EnumFormats {
        EnumFormats { idx: format }
    }

    /// Resets enumerator to list all available formats.
    pub fn reset(&mut self) -> &EnumFormats {
        self.idx = 0;
        self
    }
}

impl Iterator for EnumFormats {
    type Item = u32;

    /// Returns next format on clipboard.
    ///
    /// In case of failure (e.g. clipboard is closed) returns `None`.
    fn next(&mut self) -> Option<u32> {
        self.idx = unsafe { EnumClipboardFormats(self.idx) };

        if self.idx == 0 {
            None
        } else {
            Some(self.idx)
        }
    }

    /// Relies on `count_formats` so it is only reliable
    /// when hinting size for enumeration of all formats.
    ///
    /// Doesn't require opened clipboard.
    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, count_formats())
    }
}

macro_rules! match_format_name_big {
    ( $name:expr, $( $f:ident ),* ) => {
        match $name {
            $( formats::$f => Some(stringify!($f).to_owned()),)*
            formats::CF_GDIOBJFIRST ..= formats::CF_GDIOBJLAST => Some(format!("CF_GDIOBJ{}", $name - formats::CF_GDIOBJFIRST)),
            formats::CF_PRIVATEFIRST ..= formats::CF_PRIVATELAST => Some(format!("CF_PRIVATE{}", $name - formats::CF_PRIVATEFIRST)),
            _ => {
                let mut format_buff = [0u16; 256];
                unsafe {
                    let buff_p = format_buff.as_mut_ptr() as *mut u16;

                    match GetClipboardFormatNameW($name, buff_p, format_buff.len() as c_int) {
                        0 => None,
                        size => Some(String::from_utf16_lossy(&format_buff[..size as usize])),
                    }
                }
            }
        }
    }
}

macro_rules! match_format_name {
    ( $name:expr => $out:ident, $( $f:ident ),* ) => {
        use core::fmt::Write;

        match $name {
            $( formats::$f => {
                let _ = $out.push_str(stringify!($f));
            },)*
            formats::CF_GDIOBJFIRST ..= formats::CF_GDIOBJLAST => {
                let _ = write!($out, "CF_GDIOBJ{}", $name - formats::CF_GDIOBJFIRST);
            },
            formats::CF_PRIVATEFIRST ..= formats::CF_PRIVATELAST => {
                let _ = write!($out, "CF_PRIVATE{}", $name - formats::CF_PRIVATEFIRST);
            },
            _ => {
                let mut format_buff = [0u16; 256];
                unsafe {
                    let buff_p = format_buff.as_mut_ptr() as *mut u16;
                    match GetClipboardFormatNameW($name, buff_p, format_buff.len() as c_int) {
                        0 => return None,
                        len => match WideCharToMultiByte(CP_UTF8, 0, format_buff.as_ptr(), len, $out.as_mut_ptr() as *mut i8, $out.remaining() as i32, ptr::null(), ptr::null_mut()) {
                            0 => return None,
                            len => $out.set_len(len as _),
                        }
                    }
                }
            }
        }

        return $out.as_str()
    }
}

///Returns format name based on it's code.
///
///# Parameters:
///
///* ```format``` - clipboard format code.
///* ```out``` - temporary buffer to hold text. Buffer can be created from `&mut [u8]` and `&mut [core::mem::MaybeUninit<u8>]`
///
///# Return result:
///
///* ```Some``` Name of valid format.
///* ```None``` Format is invalid or doesn't exist or overflow happened on custom name.
pub fn format_name(format: u32, mut out: Buffer<'_>) -> Option<&'_ str> {
    match_format_name!(format => out,
                       CF_BITMAP,
                       CF_DIB,
                       CF_DIBV5,
                       CF_DIF,
                       CF_DSPBITMAP,
                       CF_DSPENHMETAFILE,
                       CF_DSPMETAFILEPICT,
                       CF_DSPTEXT,
                       CF_ENHMETAFILE,
                       CF_HDROP,
                       CF_LOCALE,
                       CF_METAFILEPICT,
                       CF_OEMTEXT,
                       CF_OWNERDISPLAY,
                       CF_PALETTE,
                       CF_PENDATA,
                       CF_RIFF,
                       CF_SYLK,
                       CF_TEXT,
                       CF_WAVE,
                       CF_TIFF,
                       CF_UNICODETEXT);
}

///Returns format name based on it's code (allocating variant suitable for big names)
///
///# Parameters:
///
///* ```format``` clipboard format code.
///
///# Return result:
///
///* ```Some``` Name of valid format.
///* ```None``` Format is invalid or doesn't exist.
pub fn format_name_big(format: u32) -> Option<String> {
    match_format_name_big!(format,
                           CF_BITMAP,
                           CF_DIB,
                           CF_DIBV5,
                           CF_DIF,
                           CF_DSPBITMAP,
                           CF_DSPENHMETAFILE,
                           CF_DSPMETAFILEPICT,
                           CF_DSPTEXT,
                           CF_ENHMETAFILE,
                           CF_HDROP,
                           CF_LOCALE,
                           CF_METAFILEPICT,
                           CF_OEMTEXT,
                           CF_OWNERDISPLAY,
                           CF_PALETTE,
                           CF_PENDATA,
                           CF_RIFF,
                           CF_SYLK,
                           CF_TEXT,
                           CF_WAVE,
                           CF_TIFF,
                           CF_UNICODETEXT)
}

#[inline]
///Registers a new clipboard format with specified name as C wide string (meaning it must have null
///char at the end).
///
///# Returns:
///
///Newly registered format identifier, if successful.
///
///# Note:
///
///- Custom format identifier is in range `0xC000...0xFFFF`.
///- Function fails if input is not null terminated string.
pub unsafe fn register_raw_format(name: &[u16]) -> Option<NonZeroU32> {
    if name.is_empty() || name[name.len()-1] != b'\0' as u16 {
        return unlikely_empty_size_result()
    }
    NonZeroU32::new(RegisterClipboardFormatW(name.as_ptr()) )
}

///Registers a new clipboard format with specified name.
///
///# Returns:
///
///Newly registered format identifier, if successful.
///
///# Note:
///
///Custom format identifier is in range `0xC000...0xFFFF`.
pub fn register_format(name: &str) -> Option<NonZeroU32> {
    let size = unsafe {
        MultiByteToWideChar(CP_UTF8, 0, name.as_ptr() as *const _, name.len() as c_int, ptr::null_mut(), 0)
    };

    if size == 0 {
        return unlikely_empty_size_result()
    }

    if size > 52 {
        let mut buffer = alloc::vec::Vec::with_capacity(size as usize);
        let size = unsafe {
            MultiByteToWideChar(CP_UTF8, 0, name.as_ptr() as *const _, name.len() as c_int, buffer.as_mut_ptr(), size)
        };
        unsafe {
            buffer.set_len(size as usize);
            buffer.push(0);
            register_raw_format(&buffer)
        }
    } else {
        let mut buffer = mem::MaybeUninit::<[u16; 52]>::zeroed();
        let size = unsafe {
            MultiByteToWideChar(CP_UTF8, 0, name.as_ptr() as *const _, name.len() as c_int, buffer.as_mut_ptr() as *mut u16, 51)
        };
        unsafe {
            ptr::write((buffer.as_mut_ptr() as *mut u16).offset(size as isize), 0);
            register_raw_format(slice::from_raw_parts(buffer.as_ptr() as *const u16, size as usize + 1))
        }
    }
}

#[inline(always)]
///Retrieves the window handle of the current owner of the clipboard.
///
///Returns `None` if clipboard is not owned.
pub fn get_owner() -> Option<ptr::NonNull::<c_void>> {
    ptr::NonNull::new(unsafe {
        GetClipboardOwner()
    })
}
