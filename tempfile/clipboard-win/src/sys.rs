use crate::types::*;

#[link(name = "kernel32", kind = "dylib")]
extern "system" {
    pub fn GlobalLock(hMem: HGLOBAL) -> LPVOID;
    pub fn GlobalUnlock(hmem: HGLOBAL) -> BOOL;
    pub fn GlobalFree(hmem: HGLOBAL) -> HGLOBAL;
    pub fn GlobalSize(hMem: HGLOBAL) -> SIZE_T;
    pub fn GlobalAlloc(uflags: c_uint, dwbytes: SIZE_T) -> HGLOBAL;
    pub fn GetCurrentThread() -> HANDLE;
    pub fn Sleep(dwMilliseconds: DWORD);

    pub fn WideCharToMultiByte(page: c_uint, flags: c_ulong, wide_str: *const u16, wide_str_len: c_int, multi_str: *mut i8, multi_str_len: c_int, default_char: *const i8, used_default_char: *mut bool) -> c_int;
    pub fn MultiByteToWideChar(CodePage: c_uint, dwFlags: DWORD, lpMultiByteStr: *const u8, cbMultiByte: c_int, lpWideCharStr: *mut u16, cchWideChar: c_int) -> c_int;
}

#[link(name = "user32", kind = "dylib")]
extern "system" {
    pub fn ReleaseDC(hWnd: HWND, hDC: HDC) -> c_int;
    pub fn GetDC(hWnd: HWND) -> HDC;

    pub fn OpenClipboard(hWnd: HWND) -> BOOL;
    pub fn CloseClipboard() -> BOOL;
    pub fn EmptyClipboard() -> BOOL;
    pub fn GetClipboardSequenceNumber() -> DWORD;
    pub fn IsClipboardFormatAvailable(format: c_uint) -> BOOL;
    pub fn GetPriorityClipboardFormat(formats: *const c_uint, size: c_int) -> BOOL;
    pub fn CountClipboardFormats() -> c_int;
    pub fn EnumClipboardFormats(format: c_uint) -> c_uint;
    pub fn GetClipboardFormatNameW(format: c_uint, lpszFormatName: *mut u16, cchMaxCount: c_int) -> c_int;
    pub fn RegisterClipboardFormatW(lpszFormat: *const u16) -> c_uint;
    pub fn GetClipboardData(uFormat: c_uint) -> HANDLE;
    pub fn SetClipboardData(uFormat: c_uint, hMem: HANDLE) -> HANDLE;
    pub fn GetClipboardOwner() -> HWND;
}

#[link(name = "shell32", kind = "dylib")]
extern "system" {
    pub fn DragQueryFileW(hDrop: HDROP, iFile: c_uint, lpszFile: *mut u16, cch: c_uint) -> c_uint;
}

#[link(name = "gdi32", kind = "dylib")]
extern "system" {
    pub fn CreateDIBitmap(hdc: HDC, pbmih: *const BITMAPINFOHEADER, flInit: DWORD, pjBits: *const c_void, pbmi: *const BITMAPINFO, iUsage: c_uint) -> HBITMAP;
    pub fn GetDIBits(hdc: HDC, hbm: HBITMAP, start: c_uint, cLines: c_uint, lpvBits: *mut c_void, lpbmi: *mut BITMAPINFO, usage: c_uint) -> c_int;
    pub fn GetObjectW(h: HANDLE, c: c_int, pv: *mut c_void) -> c_int;
}

#[link(name = "advapi32", kind = "dylib")]
extern "system" {
    pub fn ImpersonateAnonymousToken(thread_handle: HANDLE) -> BOOL;
    pub fn RevertToSelf() -> BOOL;
}
