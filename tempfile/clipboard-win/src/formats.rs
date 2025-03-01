#![allow(dead_code)]
//! Standard clipboard formats.
//!
//! Header: Winuser.h
//!
//! Description is taken from [Standard Clipboard Formats](https://msdn.microsoft.com/en-us/library/windows/desktop/ff729168%28v=vs.85%29.aspx)

use crate::{SysResult, Getter, Setter};
use crate::types::c_uint;

use core::num::NonZeroU32;

///Format trait
pub trait Format {
    ///Returns whether format is present on clipboard
    fn is_format_avail(&self) -> bool;
}

macro_rules! impl_format {
    ($($format:ident),+) => {
        $(
            impl From<$format> for u32 {
                #[inline(always)]
                fn from(value: $format) -> Self {
                    (&value).into()
                }
            }

            impl Format for $format {
                #[inline(always)]
                fn is_format_avail(&self) -> bool {
                    crate::raw::is_format_avail(self.into())
                }
            }
        )+

    };
}

///A handle to a bitmap (HBITMAP).
pub const CF_BITMAP: c_uint = 2;
///A memory object containing a <b>BITMAPINFO</b> structure followed by the bitmap bits.
pub const CF_DIB: c_uint = 8;
///A memory object containing a <b>BITMAPV5HEADER</b> structure followed by the bitmap color space
///information and the bitmap bits.
pub const CF_DIBV5: c_uint = 17;
///Software Arts' Data Interchange Format.
pub const CF_DIF: c_uint = 5;
///Bitmap display format associated with a private format. The hMem parameter must be a handle to
///data that can be displayed in bitmap format in lieu of the privately formatted data.
pub const CF_DSPBITMAP: c_uint = 0x0082;
///Enhanced metafile display format associated with a private format. The *hMem* parameter must be a
///handle to data that can be displayed in enhanced metafile format in lieu of the privately
///formatted data.
pub const CF_DSPENHMETAFILE: c_uint = 0x008E;
///Metafile-picture display format associated with a private format. The hMem parameter must be a
///handle to data that can be displayed in metafile-picture format in lieu of the privately
///formatted data.
pub const CF_DSPMETAFILEPICT: c_uint = 0x0083;
///Text display format associated with a private format. The *hMem* parameter must be a handle to
///data that can be displayed in text format in lieu of the privately formatted data.
pub const CF_DSPTEXT: c_uint = 0x0081;
///A handle to an enhanced metafile (<b>HENHMETAFILE</b>).
pub const CF_ENHMETAFILE: c_uint = 14;
///Start of a range of integer values for application-defined GDI object clipboard formats.
pub const CF_GDIOBJFIRST: c_uint = 0x0300;
///End of a range of integer values for application-defined GDI object clipboard formats.
pub const CF_GDIOBJLAST: c_uint = 0x03FF;
///A handle to type <b>HDROP</b> that identifies a list of files.
pub const CF_HDROP: c_uint = 15;
///The data is a handle to the locale identifier associated with text in the clipboard.
///
///For details see [Standart Clipboard Formats](https://msdn.microsoft.com/en-us/library/windows/desktop/ff729168%28v=vs.85%29.aspx)
pub const CF_LOCALE: c_uint = 16;
///Handle to a metafile picture format as defined by the <b>METAFILEPICT</b> structure.
pub const CF_METAFILEPICT: c_uint = 3;
///Text format containing characters in the OEM character set.
pub const CF_OEMTEXT: c_uint = 7;
///Owner-display format.
///
///For details see [Standart Clipboard Formats](https://msdn.microsoft.com/en-us/library/windows/desktop/ff729168%28v=vs.85%29.aspx)
pub const CF_OWNERDISPLAY: c_uint = 0x0080;
///Handle to a color palette.
///
///For details see [Standart Clipboard Formats](https://msdn.microsoft.com/en-us/library/windows/desktop/ff729168%28v=vs.85%29.aspx)
pub const CF_PALETTE: c_uint = 9;
///Data for the pen extensions to the Microsoft Windows for Pen Computing.
pub const CF_PENDATA: c_uint = 10;
///Start of a range of integer values for private clipboard formats.
pub const CF_PRIVATEFIRST: c_uint = 0x0200;
///End of a range of integer values for private clipboard formats.
pub const CF_PRIVATELAST: c_uint = 0x02FF;
///Represents audio data more complex than can be represented in a ```CF_WAVE``` standard wave format.
pub const CF_RIFF: c_uint = 11;
///Microsoft Symbolic Link (SYLK) format.
pub const CF_SYLK: c_uint = 4;
///ANSI text format.
pub const CF_TEXT: c_uint = 1;
///Tagged-image file format.
pub const CF_TIFF: c_uint = 6;
///UTF16 text format.
pub const CF_UNICODETEXT: c_uint = 13;
///Represents audio data in one of the standard wave formats.
pub const CF_WAVE: c_uint = 12;

#[derive(Copy, Clone)]
///Format to write/read from clipboard as raw bytes
///
///Has to be initialized with format `id`
pub struct RawData(pub c_uint);

impl<T: AsRef<[u8]>> Setter<T> for RawData {
    #[inline(always)]
    fn write_clipboard(&self, data: &T) -> SysResult<()> {
        crate::raw::set(self.0, data.as_ref())
    }
}

impl Getter<alloc::vec::Vec<u8>> for RawData {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
        crate::raw::get_vec(self.0, out)
    }
}

impl From<&RawData> for u32 {
    #[inline(always)]
    fn from(value: &RawData) -> Self {
        value.0 as _
    }
}

#[derive(Copy, Clone)]
///Format to read/write unicode string.
///
///Refer to `Getter` and `Setter`
pub struct Unicode;

impl Getter<alloc::vec::Vec<u8>> for Unicode {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
        crate::raw::get_string(out)
    }
}

impl Getter<alloc::string::String> for Unicode {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::string::String) -> SysResult<usize> {
        self.read_clipboard(unsafe { out.as_mut_vec() })
    }
}

impl<T: AsRef<str>> Setter<T> for Unicode {
    #[inline(always)]
    fn write_clipboard(&self, data: &T) -> SysResult<()> {
        crate::raw::set_string(data.as_ref())
    }
}

impl From<&Unicode> for u32 {
    #[inline(always)]
    fn from(_: &Unicode) -> Self {
        CF_UNICODETEXT
    }
}

#[derive(Copy, Clone)]
///Format for file lists (generated by drag & drop).
///
///Corresponds to `CF_HDROP`
///
///`read_clipboard` returns number of file names
pub struct FileList;

impl Getter<alloc::vec::Vec<alloc::string::String>> for FileList {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<alloc::string::String>) -> SysResult<usize> {
        crate::raw::get_file_list(out)
    }
}

#[cfg(feature = "std")]
impl Getter<alloc::vec::Vec<std::path::PathBuf>> for FileList {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<std::path::PathBuf>) -> SysResult<usize> {
        crate::raw::get_file_list_path(out)
    }
}

impl<T: AsRef<str>> Setter<[T]> for FileList {
    #[inline(always)]
    fn write_clipboard(&self, data: &[T]) -> SysResult<()> {
        crate::raw::set_file_list(data)
    }
}

impl From<&FileList> for u32 {
    #[inline(always)]
    fn from(_: &FileList) -> Self {
        CF_HDROP
    }
}

#[derive(Copy, Clone)]
///Format for bitmap images i.e. `CF_BITMAP`.
///
///Both `Getter` and `Setter` expects image as header and rgb payload
pub struct Bitmap;

impl Getter<alloc::vec::Vec<u8>> for Bitmap {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
        crate::raw::get_bitmap(out)
    }
}

impl<T: AsRef<[u8]>> Setter<T> for Bitmap {
    #[inline(always)]
    fn write_clipboard(&self, data: &T) -> SysResult<()> {
        crate::raw::set_bitmap(data.as_ref())
    }
}

impl From<&Bitmap> for u32 {
    #[inline(always)]
    fn from(_: &Bitmap) -> Self {
        CF_BITMAP
    }
}

#[derive(Copy, Clone)]
///HTML Foramt
///
///Reference: https://learn.microsoft.com/en-us/windows/win32/dataxchg/html-clipboard-format
pub struct Html(NonZeroU32);

impl Html {
    #[inline(always)]
    ///Creates new instance, if possible
    pub fn new() -> Option<Self> {
        //utf-16 "HTML Format"
        const NAME: [u16; 12] = [72, 84, 77, 76, 32, 70, 111, 114, 109, 97, 116, 0];
        unsafe {
            crate::raw::register_raw_format(&NAME).map(Self)
        }
    }

    #[inline(always)]
    ///Gets raw format code
    pub fn code(&self) -> u32 {
        self.0.get()
    }
}

impl Getter<alloc::vec::Vec<u8>> for Html {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::vec::Vec<u8>) -> SysResult<usize> {
        crate::raw::get_html(self.0.get(), out)
    }
}

impl Getter<alloc::string::String> for Html {
    #[inline(always)]
    fn read_clipboard(&self, out: &mut alloc::string::String) -> SysResult<usize> {
        crate::raw::get_html(self.0.get(), unsafe { out.as_mut_vec() })
    }
}

impl<T: AsRef<str>> Setter<T> for Html {
    #[inline(always)]
    fn write_clipboard(&self, data: &T) -> SysResult<()> {
        crate::raw::set_html(self.code(), data.as_ref())
    }
}

impl From<&Html> for u32 {
    #[inline(always)]
    fn from(value: &Html) -> Self {
        value.code()
    }
}

impl_format!(Html, Bitmap, RawData, Unicode, FileList);
