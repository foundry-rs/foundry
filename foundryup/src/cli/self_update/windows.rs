use std::{
    cell::RefCell,
    env::{consts::EXE_SUFFIX, split_paths},
    ffi::{OsStr, OsString},
    fmt,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
    process::Command,
};

use eyre::{anyhow, Context, Result};

use super::{super::errors::*, common, install_bins, InstallOpts};
use crate::{
    process,
    process::{get_process, Processor},
    utils::{self},
};

use winreg::{
    enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
    RegKey, RegValue,
};

pub(crate) fn ensure_prompt() -> Result<()> {
    writeln!(get_process().stdout(),)?;
    writeln!(get_process().stdout(), "Press the Enter key to continue.")?;
    common::read_line()?;
    Ok(())
}

/// Run by foundryup-gc-$num.exe to delete FOUNDRY_HOME
pub fn complete_windows_uninstall() -> Result<utils::ExitCode> {
    use std::process::Stdio;

    wait_for_parent()?;

    // Now that the parent has exited there are hopefully no more files open in FOUNDRY_HOME
    let foundry_home = utils::foundry_home()?;
    utils::remove_dir("foundry_home", &foundry_home)?;

    // Now, run a *system* binary to inherit the DELETE_ON_CLOSE
    // handle to *this* process, then exit. The OS will delete the gc
    // exe when it exits.
    let rm_gc_exe = OsStr::new("net");

    Command::new(rm_gc_exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context(FoundryupError::WindowsUninstallMadness)?;

    Ok(utils::ExitCode(0))
}

pub(crate) fn wait_for_parent() -> Result<()> {
    use std::{io, mem};
    use winapi::{
        shared::minwindef::DWORD,
        um::{
            handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
            processthreadsapi::{GetCurrentProcessId, OpenProcess},
            synchapi::WaitForSingleObject,
            tlhelp32::{
                CreateToolhelp32Snapshot, Process32First, Process32Next, PROCESSENTRY32,
                TH32CS_SNAPPROCESS,
            },
            winbase::{INFINITE, WAIT_OBJECT_0},
            winnt::SYNCHRONIZE,
        },
    };

    unsafe {
        // Take a snapshot of system processes, one of which is ours
        // and contains our parent's pid
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            return Err(err).context(FoundryupError::WindowsUninstallMadness)
        }

        let snapshot = scopeguard::guard(snapshot, |h| {
            let _ = CloseHandle(h);
        });

        let mut entry: PROCESSENTRY32 = mem::zeroed();
        entry.dwSize = mem::size_of::<PROCESSENTRY32>() as DWORD;

        // Iterate over system processes looking for ours
        let success = Process32First(*snapshot, &mut entry);
        if success == 0 {
            let err = io::Error::last_os_error();
            return Err(err).context(FoundryupError::WindowsUninstallMadness)
        }

        let this_pid = GetCurrentProcessId();
        while entry.th32ProcessID != this_pid {
            let success = Process32Next(*snapshot, &mut entry);
            if success == 0 {
                let err = io::Error::last_os_error();
                return Err(err).context(FoundryupError::WindowsUninstallMadness)
            }
        }

        // FIXME: Using the process ID exposes a race condition
        // wherein the parent process already exited and the OS
        // reassigned its ID.
        let parent_id = entry.th32ParentProcessID;

        // Get a handle to the parent process
        let parent = OpenProcess(SYNCHRONIZE, 0, parent_id);
        if parent.is_null() {
            // This just means the parent has already exited.
            return Ok(())
        }

        let parent = scopeguard::guard(parent, |h| {
            let _ = CloseHandle(h);
        });

        // Wait for our parent to exit
        let res = WaitForSingleObject(*parent, INFINITE);

        if res != WAIT_OBJECT_0 {
            let err = io::Error::last_os_error();
            return Err(err).context(FoundryupError::WindowsUninstallMadness)
        }
    }

    Ok(())
}

pub(crate) fn do_add_to_path() -> Result<()> {
    let new_path = _with_path_foundry_home_bin(_add_to_path)?;
    _apply_new_path(new_path)
}

fn _apply_new_path(new_path: Option<Vec<u16>>) -> Result<()> {
    use std::ptr;
    use winapi::{
        shared::minwindef::*,
        um::winuser::{SendMessageTimeoutA, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE},
    };

    let new_path = match new_path {
        Some(new_path) => new_path,
        None => return Ok(()), // No need to set the path
    };

    let root = RegKey::predef(HKEY_CURRENT_USER);
    let environment = root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)?;

    if new_path.is_empty() {
        environment.delete_value("PATH")?;
    } else {
        let reg_value =
            RegValue { bytes: to_winreg_bytes(new_path), vtype: RegType::REG_EXPAND_SZ };
        environment.set_raw_value("PATH", &reg_value)?;
    }

    // Tell other processes to update their environment
    #[allow(clippy::unnecessary_cast)]
    unsafe {
        SendMessageTimeoutA(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0 as WPARAM,
            "Environment\0".as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            5000,
            ptr::null_mut(),
        );
    }

    Ok(())
}

// Get the windows PATH variable out of the registry as a String. If
// this returns None then the PATH variable is not a string and we
// should not mess with it.
fn get_windows_path_var() -> Result<Option<Vec<u16>>> {
    use std::io;

    let root = RegKey::predef(HKEY_CURRENT_USER);
    let environment = root
        .open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE)
        .context("Failed opening Environment key")?;

    let reg_value = environment.get_raw_value("PATH");
    match reg_value {
        Ok(val) => {
            if let Some(s) = from_winreg_value(&val) {
                Ok(Some(s))
            } else {
                warn!(
                    "the registry key HKEY_CURRENT_USER\\Environment\\PATH is not a string. \
                       Not modifying the PATH variable"
                );
                Ok(None)
            }
        }
        Err(ref e) if e.kind() == io::ErrorKind::NotFound => Ok(Some(Vec::new())),
        Err(e) => Err(e).context(FoundryupError::WindowsUninstallMadness),
    }
}

// Returns None if the existing old_path does not need changing, otherwise
// prepends the path_str to old_path, handling empty old_path appropriately.
fn _add_to_path(old_path: Vec<u16>, path_str: Vec<u16>) -> Option<Vec<u16>> {
    if old_path.is_empty() {
        Some(path_str)
    } else if old_path.windows(path_str.len()).any(|path| path == path_str) {
        None
    } else {
        let mut new_path = path_str;
        new_path.push(b';' as u16);
        new_path.extend_from_slice(&old_path);
        Some(new_path)
    }
}

// Returns None if the existing old_path does not need changing
fn _remove_from_path(old_path: Vec<u16>, path_str: Vec<u16>) -> Option<Vec<u16>> {
    let idx = old_path.windows(path_str.len()).position(|path| path == path_str)?;
    // If there's a trailing semicolon (likely, since we probably added one
    // during install), include that in the substring to remove. We don't search
    // for that to find the string, because if its the last string in the path,
    // there may not be.
    let mut len = path_str.len();
    if old_path.get(idx + path_str.len()) == Some(&(b';' as u16)) {
        len += 1;
    }

    let mut new_path = old_path[..idx].to_owned();
    new_path.extend_from_slice(&old_path[idx + len..]);
    // Don't leave a trailing ; though, we don't want an empty string in the
    // path.
    if new_path.last() == Some(&(b';' as u16)) {
        new_path.pop();
    }
    Some(new_path)
}

fn _with_path_foundry_home_bin<F>(f: F) -> Result<Option<Vec<u16>>>
where
    F: FnOnce(Vec<u16>, Vec<u16>) -> Option<Vec<u16>>,
{
    let windows_path = get_windows_path_var()?;
    let mut path_str = utils::foundry_home()?;
    path_str.push("bin");
    Ok(windows_path
        .and_then(|old_path| f(old_path, OsString::from(path_str).encode_wide().collect())))
}

pub(crate) fn do_remove_from_path() -> Result<()> {
    let new_path = _with_path_foundry_home_bin(_remove_from_path)?;
    _apply_new_path(new_path)
}

const FOUNDRYUP_UNINSTALL_ENTRY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Foundryup";

pub(crate) fn do_add_to_programs() -> Result<()> {
    use std::path::PathBuf;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(FOUNDRYUP_UNINSTALL_ENTRY)
        .context("Failed creating uninstall key")?
        .0;

    // Don't overwrite registry if Foundryup is already installed
    let prev = key.get_raw_value("UninstallString").map(|val| from_winreg_value(&val));
    if let Ok(Some(s)) = prev {
        let mut path = PathBuf::from(OsString::from_wide(&s));
        path.pop();
        if path.exists() {
            return Ok(())
        }
    }

    let mut path = utils::foundry_home()?;
    path.push("bin\\foundryup.exe");
    let mut uninstall_cmd = OsString::from("\"");
    uninstall_cmd.push(path);
    uninstall_cmd.push("\" self uninstall");

    let reg_value = RegValue {
        bytes: to_winreg_bytes(uninstall_cmd.encode_wide().collect()),
        vtype: RegType::REG_SZ,
    };

    key.set_raw_value("UninstallString", &reg_value).context("Failed to set uninstall string")?;
    key.set_value("DisplayName", &"Foundryup: the Foundry toolchain installer")
        .context("Failed to set display name")?;

    Ok(())
}

pub(crate) fn do_remove_from_programs() -> Result<()> {
    match RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(FOUNDRYUP_UNINSTALL_ENTRY) {
        Ok(()) => Ok(()),
        Err(ref e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow!(e)),
    }
}

/// Convert a vector UCS-2 chars to a null-terminated UCS-2 string in bytes
pub(crate) fn to_winreg_bytes(mut v: Vec<u16>) -> Vec<u8> {
    v.push(0);
    unsafe { std::slice::from_raw_parts(v.as_ptr().cast::<u8>(), v.len() * 2).to_vec() }
}

/// This is used to decode the value of HKCU\Environment\PATH. If that key is
/// not REG_SZ | REG_EXPAND_SZ then this returns None. The winreg library itself
/// does a lossy unicode conversion.
pub(crate) fn from_winreg_value(val: &winreg::RegValue) -> Option<Vec<u16>> {
    use std::slice;

    match val.vtype {
        RegType::REG_SZ | RegType::REG_EXPAND_SZ => {
            // Copied from winreg
            let mut words = unsafe {
                #[allow(clippy::cast_ptr_alignment)]
                slice::from_raw_parts(val.bytes.as_ptr().cast::<u16>(), val.bytes.len() / 2)
                    .to_owned()
            };
            while words.last() == Some(&0) {
                words.pop();
            }
            Some(words)
        }
        _ => None,
    }
}

pub(crate) fn run_update(setup_path: &Path) -> Result<utils::ExitCode> {
    Command::new(setup_path).arg("--self-replace").spawn().context("unable to run updater")?;

    Ok(utils::ExitCode(0))
}

pub(crate) fn self_replace() -> Result<utils::ExitCode> {
    wait_for_parent()?;
    install_bins()?;

    Ok(utils::ExitCode(0))
}

// The last step of uninstallation is to delete *this binary*,
// foundryup.exe and the FOUNDRY_HOME that contains it. On Unix, this
// works fine. On Windows you can't delete files while they are open,
// like when they are running.
//
// Here's what we're going to do:
// - Copy foundryup.exe to a temporary file in FOUNDRY_HOME/../foundryup-gc-$random.exe.
// - Open the gc exe with the FILE_FLAG_DELETE_ON_CLOSE and FILE_SHARE_DELETE flags. This is going
//   to be the last file to remove, and the OS is going to do it for us. This file is opened as
//   inheritable so that subsequent processes created with the option to inherit handles will also
//   keep them open.
// - Run the gc exe, which waits for the original foundryup.exe process to close, then deletes
//   FOUNDRY_HOME. This process has inherited a FILE_FLAG_DELETE_ON_CLOSE handle to itself.
// - Finally, spawn yet another system binary with the inherit handles flag, so *it* inherits the
//   FILE_FLAG_DELETE_ON_CLOSE handle to the gc exe. If the gc exe exits before the system exe then
//   at last it will be deleted when the handle closes.
//
// This is the DELETE_ON_CLOSE method from
// https://www.catch22.net/tuts/win32/self-deleting-executables
//
// ... which doesn't actually work because Windows won't really
// delete a FILE_FLAG_DELETE_ON_CLOSE process when it exits.
//
// .. augmented with this SO answer
// https://stackoverflow.com/questions/10319526/understanding-a-self-deleting-program-in-c
pub(crate) fn delete_foundry_home() -> Result<()> {
    use std::{io, mem, ptr, thread, time::Duration};
    use winapi::{
        shared::minwindef::DWORD,
        um::{
            fileapi::{CreateFileW, OPEN_EXISTING},
            handleapi::{CloseHandle, INVALID_HANDLE_VALUE},
            minwinbase::SECURITY_ATTRIBUTES,
            winbase::FILE_FLAG_DELETE_ON_CLOSE,
            winnt::{FILE_SHARE_DELETE, FILE_SHARE_READ, GENERIC_READ},
        },
    };

    // FOUNDRY_HOME, hopefully empty except for bin/foundryup.exe
    let foundry_home = utils::foundry_home()?;
    // The foundryup.exe bin
    let foundryup_path = foundry_home.join(&format!("bin/foundryup{}", EXE_SUFFIX));

    // The directory containing FOUNDRY_HOME
    let work_path = foundry_home.parent().expect("FOUNDRY_HOME doesn't have a parent?");

    // Generate a unique name for the files we're about to move out
    // of FOUNDRY_HOME.
    let numbah: u32 = rand::random();
    let gc_exe = work_path.join(&format!("foundryup-gc-{:x}.exe", numbah));
    // Copy foundryup (probably this process's exe) to the gc exe
    utils::copy_file(&foundryup_path, &gc_exe)?;
    let gc_exe_win: Vec<_> = gc_exe.as_os_str().encode_wide().chain(Some(0)).collect();

    // Make the sub-process opened by gc exe inherit its attribute.
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as DWORD,
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: 1,
    };

    let _g = unsafe {
        // Open an inheritable handle to the gc exe marked
        // FILE_FLAG_DELETE_ON_CLOSE.
        let gc_handle = CreateFileW(
            gc_exe_win.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_DELETE,
            &mut sa,
            OPEN_EXISTING,
            FILE_FLAG_DELETE_ON_CLOSE,
            ptr::null_mut(),
        );

        if gc_handle == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            return Err(err).context(FoundryupError::WindowsUninstallMadness)
        }

        scopeguard::guard(gc_handle, |h| {
            let _ = CloseHandle(h);
        })
    };

    Command::new(gc_exe).spawn().context(FoundryupError::WindowsUninstallMadness)?;

    // The catch 22 article says we must sleep here to give
    // Windows a chance to bump the processes file reference
    // count. acrichto though is in disbelief and *demanded* that
    // we not insert a sleep. If Windows failed to uninstall
    // correctly it is because of him.

    // (.. and months later acrichto owes me a beer).
    thread::sleep(Duration::from_millis(100));

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::windows::ffi::OsStrExt};

    use winreg::{
        enums::{RegType, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE},
        RegKey, RegValue,
    };

    use crate::{currentprocess, test::with_saved_path};

    fn wide(str: &str) -> Vec<u16> {
        OsString::from(str).encode_wide().collect()
    }

    #[test]
    fn windows_install_does_not_add_path_twice() {
        assert_eq!(
            None,
            super::_add_to_path(
                wide(r"c:\users\example\.foundry\bin;foo"),
                wide(r"c:\users\example\.foundry\bin")
            )
        );
    }

    #[test]
    fn windows_handle_non_unicode_path() {
        let initial_path = vec![
            0xD800, // leading surrogate
            0x0101, // bogus trailing surrogate
            0x0000, // null
        ];
        let foundry_home = wide(r"c:\users\example\.foundry\bin");
        let final_path = [&foundry_home, &[b';' as u16][..], &initial_path].join(&[][..]);

        assert_eq!(
            &final_path,
            &super::_add_to_path(initial_path.clone(), foundry_home.clone(),).unwrap()
        );
        assert_eq!(&initial_path, &super::_remove_from_path(final_path, foundry_home,).unwrap());
    }

    #[test]
    fn windows_path_regkey_type() {
        // per issue #261, setting PATH should use REG_EXPAND_SZ.
        let tp = Box::new(currentprocess::TestProcess::default());
        with_saved_path(&|| {
            currentprocess::with(tp.clone(), || {
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let environment =
                    root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).unwrap();
                environment.delete_value("PATH").unwrap();

                {
                    assert_eq!((), super::_apply_new_path(Some(wide("foo"))).unwrap());
                }
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let environment =
                    root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).unwrap();
                let path = environment.get_raw_value("PATH").unwrap();
                assert_eq!(path.vtype, RegType::REG_EXPAND_SZ);
                assert_eq!(super::to_winreg_bytes(wide("foo")), &path.bytes[..]);
            })
        });
    }

    #[test]
    fn windows_path_delete_key_when_empty() {
        use std::io;
        // during uninstall the PATH key may end up empty; if so we should
        // delete it.
        let tp = Box::new(currentprocess::TestProcess::default());
        with_saved_path(&|| {
            currentprocess::with(tp.clone(), || {
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let environment =
                    root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).unwrap();
                environment
                    .set_raw_value(
                        "PATH",
                        &RegValue {
                            bytes: super::to_winreg_bytes(wide("foo")),
                            vtype: RegType::REG_EXPAND_SZ,
                        },
                    )
                    .unwrap();

                {
                    assert_eq!((), super::_apply_new_path(Some(Vec::new())).unwrap());
                }
                let reg_value = environment.get_raw_value("PATH");
                match reg_value {
                    Ok(_) => panic!("key not deleted"),
                    Err(ref e) if e.kind() == io::ErrorKind::NotFound => {}
                    Err(ref e) => panic!("error {}", e),
                }
            })
        });
    }

    #[test]
    fn windows_doesnt_mess_with_a_non_string_path() {
        // This writes an error, so we want a sink for it.
        let tp = Box::new(currentprocess::TestProcess {
            vars: [("HOME".to_string(), "/unused".to_string())].iter().cloned().collect(),
            ..Default::default()
        });
        with_saved_path(&|| {
            currentprocess::with(tp.clone(), || {
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let environment =
                    root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).unwrap();
                let reg_value = RegValue { bytes: vec![0x12, 0x34], vtype: RegType::REG_BINARY };
                environment.set_raw_value("PATH", &reg_value).unwrap();
                // Ok(None) signals no change to the PATH setting layer
                assert_eq!(
                    None,
                    super::_with_path_foundry_home_bin(|_, _| panic!("called")).unwrap()
                );
            })
        });
        assert_eq!(
            r"warning: the registry key HKEY_CURRENT_USER\Environment\PATH is not a string. Not modifying the PATH variable
",
            String::from_utf8(tp.get_stderr()).unwrap()
        );
    }

    #[test]
    fn windows_treat_missing_path_as_empty() {
        // during install the PATH key may be missing; treat it as empty
        let tp = Box::new(currentprocess::TestProcess::default());
        with_saved_path(&|| {
            currentprocess::with(tp.clone(), || {
                let root = RegKey::predef(HKEY_CURRENT_USER);
                let environment =
                    root.open_subkey_with_flags("Environment", KEY_READ | KEY_WRITE).unwrap();
                environment.delete_value("PATH").unwrap();

                assert_eq!(Some(Vec::new()), super::get_windows_path_var().unwrap());
            })
        });
    }

    #[test]
    fn windows_uninstall_removes_semicolon_from_path_prefix() {
        assert_eq!(
            wide("foo"),
            super::_remove_from_path(
                wide(r"c:\users\example\.foundry\bin;foo"),
                wide(r"c:\users\example\.foundry\bin"),
            )
            .unwrap()
        )
    }

    #[test]
    fn windows_uninstall_removes_semicolon_from_path_suffix() {
        assert_eq!(
            wide("foo"),
            super::_remove_from_path(
                wide(r"foo;c:\users\example\.foundry\bin"),
                wide(r"c:\users\example\.foundry\bin"),
            )
            .unwrap()
        )
    }
}
