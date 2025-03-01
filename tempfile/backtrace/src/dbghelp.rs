//! A module to assist in managing dbghelp bindings on Windows
//!
//! Backtraces on Windows (at least for MSVC) are largely powered through
//! `dbghelp.dll` and the various functions that it contains. These functions
//! are currently loaded *dynamically* rather than linking to `dbghelp.dll`
//! statically. This is currently done by the standard library (and is in theory
//! required there), but is an effort to help reduce the static dll dependencies
//! of a library since backtraces are typically pretty optional. That being
//! said, `dbghelp.dll` almost always successfully loads on Windows.
//!
//! Note though that since we're loading all this support dynamically we can't
//! actually use the raw definitions in `winapi`, but rather we need to define
//! the function pointer types ourselves and use that. We don't really want to
//! be in the business of duplicating winapi, so we have a Cargo feature
//! `verify-winapi` which asserts that all bindings match those in winapi and
//! this feature is enabled on CI.
//!
//! Finally, you'll note here that the dll for `dbghelp.dll` is never unloaded,
//! and that's currently intentional. The thinking is that we can globally cache
//! it and use it between calls to the API, avoiding expensive loads/unloads. If
//! this is a problem for leak detectors or something like that we can cross the
//! bridge when we get there.

#![allow(non_snake_case)]

use alloc::vec::Vec;

use super::windows::*;
use core::mem;
use core::ptr;
use core::slice;

// Work around `SymGetOptions` and `SymSetOptions` not being present in winapi
// itself. Otherwise this is only used when we're double-checking types against
// winapi.
#[cfg(feature = "verify-winapi")]
mod dbghelp {
    use crate::windows::*;
    pub use winapi::um::dbghelp::{
        StackWalk64, StackWalkEx, SymFromAddrW, SymFunctionTableAccess64, SymGetLineFromAddrW64,
        SymGetModuleBase64, SymGetOptions, SymInitializeW, SymSetOptions,
    };

    extern "system" {
        // Not defined in winapi yet
        pub fn SymFromInlineContextW(
            hProcess: HANDLE,
            Address: DWORD64,
            InlineContext: ULONG,
            Displacement: PDWORD64,
            Symbol: PSYMBOL_INFOW,
        ) -> BOOL;
        pub fn SymGetLineFromInlineContextW(
            hProcess: HANDLE,
            dwAddr: DWORD64,
            InlineContext: ULONG,
            qwModuleBaseAddress: DWORD64,
            pdwDisplacement: PDWORD,
            Line: PIMAGEHLP_LINEW64,
        ) -> BOOL;
        pub fn SymAddrIncludeInlineTrace(hProcess: HANDLE, Address: DWORD64) -> DWORD;
        pub fn SymQueryInlineTrace(
            hProcess: HANDLE,
            StartAddress: DWORD64,
            StartContext: DWORD,
            StartRetAddress: DWORD64,
            CurAddress: DWORD64,
            CurContext: LPDWORD,
            CurFrameIndex: LPDWORD,
        ) -> BOOL;
        pub fn SymGetSearchPathW(
            hprocess: HANDLE,
            searchpatha: PWSTR,
            searchpathlength: DWORD,
        ) -> BOOL;
        pub fn SymSetSearchPathW(hprocess: HANDLE, searchpatha: PCWSTR) -> BOOL;
        pub fn EnumerateLoadedModulesW64(
            hprocess: HANDLE,
            enumloadedmodulescallback: PENUMLOADED_MODULES_CALLBACKW64,
            usercontext: PVOID,
        ) -> BOOL;
    }

    pub fn assert_equal_types<T>(a: T, _b: T) -> T {
        a
    }
}

// This macro is used to define a `Dbghelp` structure which internally contains
// all the function pointers that we might load.
macro_rules! dbghelp {
    (extern "system" {
        $(fn $name:ident($($arg:ident: $argty:ty),*) -> $ret: ty;)*
    }) => (
        pub struct Dbghelp {
            /// The loaded DLL for `dbghelp.dll`
            dll: HMODULE,

            // Each function pointer for each function we might use
            $($name: usize,)*
        }

        static mut DBGHELP: Dbghelp = Dbghelp {
            // Initially we haven't loaded the DLL
            dll: ptr::null_mut(),
            // Initially all functions are set to zero to say they need to be
            // dynamically loaded.
            $($name: 0,)*
        };

        // Convenience typedef for each function type.
        $(pub type $name = unsafe extern "system" fn($($argty),*) -> $ret;)*

        impl Dbghelp {
            /// Attempts to open `dbghelp.dll`. Returns success if it works or
            /// error if `LoadLibraryW` fails.
            ///
            /// Panics if library is already loaded.
            fn ensure_open(&mut self) -> Result<(), ()> {
                if !self.dll.is_null() {
                    return Ok(())
                }
                let lib = b"dbghelp.dll\0";
                unsafe {
                    self.dll = LoadLibraryA(lib.as_ptr().cast::<i8>());
                    if self.dll.is_null() {
                        Err(())
                    }  else {
                        Ok(())
                    }
                }
            }

            // Function for each method we'd like to use. When called it will
            // either read the cached function pointer or load it and return the
            // loaded value. Loads are asserted to succeed.
            $(pub fn $name(&mut self) -> Option<$name> {
                unsafe {
                    if self.$name == 0 {
                        let name = concat!(stringify!($name), "\0");
                        self.$name = self.symbol(name.as_bytes())?;
                    }
                    let ret = mem::transmute::<usize, $name>(self.$name);
                    #[cfg(feature = "verify-winapi")]
                    dbghelp::assert_equal_types(ret, dbghelp::$name);
                    Some(ret)
                }
            })*

            fn symbol(&self, symbol: &[u8]) -> Option<usize> {
                unsafe {
                    match GetProcAddress(self.dll, symbol.as_ptr().cast()) as usize {
                        0 => None,
                        n => Some(n),
                    }
                }
            }
        }

        // Convenience proxy to use the cleanup locks to reference dbghelp
        // functions.
        #[allow(dead_code)]
        impl Init {
            $(pub fn $name(&self) -> $name {
                unsafe {
                    DBGHELP.$name().unwrap()
                }
            })*

            pub fn dbghelp(&self) -> *mut Dbghelp {
                unsafe {
                    ptr::addr_of_mut!(DBGHELP)
                }
            }
        }
    )

}

const SYMOPT_DEFERRED_LOADS: DWORD = 0x00000004;

dbghelp! {
    extern "system" {
        fn SymGetOptions() -> DWORD;
        fn SymSetOptions(options: DWORD) -> DWORD;
        fn SymInitializeW(
            handle: HANDLE,
            path: PCWSTR,
            invade: BOOL
        ) -> BOOL;
        fn SymGetSearchPathW(
            hprocess: HANDLE,
            searchpatha: PWSTR,
            searchpathlength: DWORD
        ) -> BOOL;
        fn SymSetSearchPathW(
            hprocess: HANDLE,
            searchpatha: PCWSTR
        ) -> BOOL;
        fn EnumerateLoadedModulesW64(
            hprocess: HANDLE,
            enumloadedmodulescallback: PENUMLOADED_MODULES_CALLBACKW64,
            usercontext: PVOID
        ) -> BOOL;
        fn StackWalk64(
            MachineType: DWORD,
            hProcess: HANDLE,
            hThread: HANDLE,
            StackFrame: LPSTACKFRAME64,
            ContextRecord: PVOID,
            ReadMemoryRoutine: PREAD_PROCESS_MEMORY_ROUTINE64,
            FunctionTableAccessRoutine: PFUNCTION_TABLE_ACCESS_ROUTINE64,
            GetModuleBaseRoutine: PGET_MODULE_BASE_ROUTINE64,
            TranslateAddress: PTRANSLATE_ADDRESS_ROUTINE64
        ) -> BOOL;
        fn SymFunctionTableAccess64(
            hProcess: HANDLE,
            AddrBase: DWORD64
        ) -> PVOID;
        fn SymGetModuleBase64(
            hProcess: HANDLE,
            AddrBase: DWORD64
        ) -> DWORD64;
        fn StackWalkEx(
            MachineType: DWORD,
            hProcess: HANDLE,
            hThread: HANDLE,
            StackFrame: LPSTACKFRAME_EX,
            ContextRecord: PVOID,
            ReadMemoryRoutine: PREAD_PROCESS_MEMORY_ROUTINE64,
            FunctionTableAccessRoutine: PFUNCTION_TABLE_ACCESS_ROUTINE64,
            GetModuleBaseRoutine: PGET_MODULE_BASE_ROUTINE64,
            TranslateAddress: PTRANSLATE_ADDRESS_ROUTINE64,
            Flags: DWORD
        ) -> BOOL;
        fn SymFromInlineContextW(
            hProcess: HANDLE,
            Address: DWORD64,
            InlineContext: ULONG,
            Displacement: PDWORD64,
            Symbol: PSYMBOL_INFOW
        ) -> BOOL;
        fn SymGetLineFromInlineContextW(
            hProcess: HANDLE,
            dwAddr: DWORD64,
            InlineContext: ULONG,
            qwModuleBaseAddress: DWORD64,
            pdwDisplacement: PDWORD,
            Line: PIMAGEHLP_LINEW64
        ) -> BOOL;
        fn SymAddrIncludeInlineTrace(
            hProcess: HANDLE,
            Address: DWORD64
        ) -> DWORD;
        fn SymQueryInlineTrace(
            hProcess: HANDLE,
            StartAddress: DWORD64,
            StartContext: DWORD,
            StartRetAddress: DWORD64,
            CurAddress: DWORD64,
            CurContext: LPDWORD,
            CurFrameIndex: LPDWORD
        ) -> BOOL;
        fn SymFromAddrW(
            hProcess: HANDLE,
            Address: DWORD64,
            Displacement: PDWORD64,
            Symbol: PSYMBOL_INFOW
        ) -> BOOL;
        fn SymGetLineFromAddrW64(
            hProcess: HANDLE,
            dwAddr: DWORD64,
            pdwDisplacement: PDWORD,
            Line: PIMAGEHLP_LINEW64
        ) -> BOOL;
    }
}

pub struct Init {
    lock: HANDLE,
}

/// Initialize all support necessary to access `dbghelp` API functions from this
/// crate.
///
/// Note that this function is **safe**, it internally has its own
/// synchronization. Also note that it is safe to call this function multiple
/// times recursively.
pub fn init() -> Result<Init, ()> {
    use core::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    // Helper function for generating a name that's unique to the process.
    fn mutex_name() -> [u8; 33] {
        let mut name: [u8; 33] = *b"Local\\RustBacktraceMutex00000000\0";
        let mut id = unsafe { GetCurrentProcessId() };
        // Quick and dirty no alloc u32 to hex.
        let mut index = name.len() - 1;
        while id > 0 {
            name[index - 1] = match (id & 0xF) as u8 {
                h @ 0..=9 => b'0' + h,
                h => b'A' + (h - 10),
            };
            id >>= 4;
            index -= 1;
        }
        name
    }

    unsafe {
        // First thing we need to do is to synchronize this function. This can
        // be called concurrently from other threads or recursively within one
        // thread. Note that it's trickier than that though because what we're
        // using here, `dbghelp`, *also* needs to be synchronized with all other
        // callers to `dbghelp` in this process.
        //
        // Typically there aren't really that many calls to `dbghelp` within the
        // same process and we can probably safely assume that we're the only
        // ones accessing it. There is, however, one primary other user we have
        // to worry about which is ironically ourselves, but in the standard
        // library. The Rust standard library depends on this crate for
        // backtrace support, and this crate also exists on crates.io. This
        // means that if the standard library is printing a panic backtrace it
        // may race with this crate coming from crates.io, causing segfaults.
        //
        // To help solve this synchronization problem we employ a
        // Windows-specific trick here (it is, after all, a Windows-specific
        // restriction about synchronization). We create a *session-local* named
        // mutex to protect this call. The intention here is that the standard
        // library and this crate don't have to share Rust-level APIs to
        // synchronize here but can instead work behind the scenes to make sure
        // they're synchronizing with one another. That way when this function
        // is called through the standard library or through crates.io we can be
        // sure that the same mutex is being acquired.
        //
        // So all of that is to say that the first thing we do here is we
        // atomically create a `HANDLE` which is a named mutex on Windows. We
        // synchronize a bit with other threads sharing this function
        // specifically and ensure that only one handle is created per instance
        // of this function. Note that the handle is never closed once it's
        // stored in the global.
        //
        // After we've actually go the lock we simply acquire it, and our `Init`
        // handle we hand out will be responsible for dropping it eventually.
        static LOCK: AtomicUsize = AtomicUsize::new(0);
        let mut lock = LOCK.load(SeqCst);
        if lock == 0 {
            let name = mutex_name();
            lock = CreateMutexA(ptr::null_mut(), 0, name.as_ptr().cast::<i8>()) as usize;
            if lock == 0 {
                return Err(());
            }
            if let Err(other) = LOCK.compare_exchange(0, lock, SeqCst, SeqCst) {
                debug_assert!(other != 0);
                CloseHandle(lock as HANDLE);
                lock = other;
            }
        }
        debug_assert!(lock != 0);
        let lock = lock as HANDLE;
        let r = WaitForSingleObjectEx(lock, INFINITE, FALSE);
        debug_assert_eq!(r, 0);
        let ret = Init { lock };

        // Ok, phew! Now that we're all safely synchronized, let's actually
        // start processing everything. First up we need to ensure that
        // `dbghelp.dll` is actually loaded in this process. We do this
        // dynamically to avoid a static dependency. This has historically been
        // done to work around weird linking issues and is intended at making
        // binaries a bit more portable since this is largely just a debugging
        // utility.
        //
        // Once we've opened `dbghelp.dll` we need to call some initialization
        // functions in it, and that's detailed more below. We only do this
        // once, though, so we've got a global boolean indicating whether we're
        // done yet or not.
        DBGHELP.ensure_open()?;

        static mut INITIALIZED: bool = false;
        if INITIALIZED {
            return Ok(ret);
        }

        let orig = DBGHELP.SymGetOptions().unwrap()();

        // Ensure that the `SYMOPT_DEFERRED_LOADS` flag is set, because
        // according to MSVC's own docs about this: "This is the fastest, most
        // efficient way to use the symbol handler.", so let's do that!
        DBGHELP.SymSetOptions().unwrap()(orig | SYMOPT_DEFERRED_LOADS);

        // Actually initialize symbols with MSVC. Note that this can fail, but we
        // ignore it. There's not a ton of prior art for this per se, but LLVM
        // internally seems to ignore the return value here and one of the
        // sanitizer libraries in LLVM prints a scary warning if this fails but
        // basically ignores it in the long run.
        //
        // One case this comes up a lot for Rust is that the standard library and
        // this crate on crates.io both want to compete for `SymInitializeW`. The
        // standard library historically wanted to initialize then cleanup most of
        // the time, but now that it's using this crate it means that someone will
        // get to initialization first and the other will pick up that
        // initialization.
        DBGHELP.SymInitializeW().unwrap()(GetCurrentProcess(), ptr::null_mut(), TRUE);

        // The default search path for dbghelp will only look in the current working
        // directory and (possibly) `_NT_SYMBOL_PATH` and `_NT_ALT_SYMBOL_PATH`.
        // However, we also want to look in the directory of the executable
        // and each DLL that is loaded. To do this, we need to update the search path
        // to include these directories.
        //
        // See https://learn.microsoft.com/cpp/build/reference/pdbpath for an
        // example of where symbols are usually searched for.
        let mut search_path_buf = Vec::new();
        search_path_buf.resize(1024, 0);

        // Prefill the buffer with the current search path.
        if DBGHELP.SymGetSearchPathW().unwrap()(
            GetCurrentProcess(),
            search_path_buf.as_mut_ptr(),
            search_path_buf.len() as _,
        ) == TRUE
        {
            // Trim the buffer to the actual length of the string.
            let len = lstrlenW(search_path_buf.as_mut_ptr());
            assert!(len >= 0);
            search_path_buf.truncate(len as usize);
        } else {
            // If getting the search path fails, at least include the current directory.
            search_path_buf.clear();
            search_path_buf.push(utf16_char('.'));
            search_path_buf.push(utf16_char(';'));
        }

        let mut search_path = SearchPath::new(search_path_buf);

        // Update the search path to include the directory of the executable and each DLL.
        DBGHELP.EnumerateLoadedModulesW64().unwrap()(
            GetCurrentProcess(),
            Some(enum_loaded_modules_callback),
            ((&mut search_path) as *mut SearchPath) as *mut c_void,
        );

        let new_search_path = search_path.finalize();

        // Set the new search path.
        DBGHELP.SymSetSearchPathW().unwrap()(GetCurrentProcess(), new_search_path.as_ptr());

        INITIALIZED = true;
        Ok(ret)
    }
}

struct SearchPath {
    search_path_utf16: Vec<u16>,
}

fn utf16_char(c: char) -> u16 {
    let buf = &mut [0u16; 2];
    let buf = c.encode_utf16(buf);
    assert!(buf.len() == 1);
    buf[0]
}

impl SearchPath {
    fn new(initial_search_path: Vec<u16>) -> Self {
        Self {
            search_path_utf16: initial_search_path,
        }
    }

    /// Add a path to the search path if it is not already present.
    fn add(&mut self, path: &[u16]) {
        let sep = utf16_char(';');

        // We could deduplicate in a case-insensitive way, but case-sensitivity
        // can be configured by directory on Windows, so let's not do that.
        // https://learn.microsoft.com/windows/wsl/case-sensitivity
        if !self
            .search_path_utf16
            .split(|&c| c == sep)
            .any(|p| p == path)
        {
            if self.search_path_utf16.last() != Some(&sep) {
                self.search_path_utf16.push(sep);
            }
            self.search_path_utf16.extend_from_slice(path);
        }
    }

    fn finalize(mut self) -> Vec<u16> {
        // Add a null terminator.
        self.search_path_utf16.push(0);
        self.search_path_utf16
    }
}

extern "system" fn enum_loaded_modules_callback(
    module_name: PCWSTR,
    _: DWORD64,
    _: ULONG,
    user_context: PVOID,
) -> BOOL {
    // `module_name` is an absolute path like `C:\path\to\module.dll`
    // or `C:\path\to\module.exe`
    let len: usize = unsafe { lstrlenW(module_name).try_into().unwrap() };

    if len == 0 {
        // This should not happen, but if it does, we can just ignore it.
        return TRUE;
    }

    let module_name = unsafe { slice::from_raw_parts(module_name, len) };
    let path_sep = utf16_char('\\');
    let alt_path_sep = utf16_char('/');

    let Some(end_of_directory) = module_name
        .iter()
        .rposition(|&c| c == path_sep || c == alt_path_sep)
    else {
        // `module_name` being an absolute path, it should always contain at least one
        // path separator. If not, there is nothing we can do.
        return TRUE;
    };

    let search_path = unsafe { &mut *(user_context as *mut SearchPath) };
    search_path.add(&module_name[..end_of_directory]);

    TRUE
}

impl Drop for Init {
    fn drop(&mut self) {
        unsafe {
            let r = ReleaseMutex(self.lock);
            debug_assert!(r != 0);
        }
    }
}
