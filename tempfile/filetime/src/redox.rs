use crate::FileTime;
use std::fs::{self, File};
use std::io;
use std::os::unix::prelude::*;
use std::path::Path;

use libredox::{
    call, errno,
    error::{Error, Result},
    flag, Fd,
};

pub fn set_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    set_file_times_redox(fd.raw(), atime, mtime)
}

pub fn set_file_mtime(p: &Path, mtime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    let st = fd.stat()?;

    set_file_times_redox(
        fd.raw(),
        FileTime {
            seconds: st.st_atime as i64,
            nanos: st.st_atime_nsec as u32,
        },
        mtime,
    )?;
    Ok(())
}

pub fn set_file_atime(p: &Path, atime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, 0)?;
    let st = fd.stat()?;

    set_file_times_redox(
        fd.raw(),
        atime,
        FileTime {
            seconds: st.st_mtime as i64,
            nanos: st.st_mtime_nsec as u32,
        },
    )?;
    Ok(())
}

pub fn set_symlink_file_times(p: &Path, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    let fd = open_redox(p, flag::O_NOFOLLOW)?;
    set_file_times_redox(fd.raw(), atime, mtime)?;
    Ok(())
}

pub fn set_file_handle_times(
    f: &File,
    atime: Option<FileTime>,
    mtime: Option<FileTime>,
) -> io::Result<()> {
    let (atime1, mtime1) = match (atime, mtime) {
        (Some(a), Some(b)) => (a, b),
        (None, None) => return Ok(()),
        (Some(a), None) => {
            let meta = f.metadata()?;
            (a, FileTime::from_last_modification_time(&meta))
        }
        (None, Some(b)) => {
            let meta = f.metadata()?;
            (FileTime::from_last_access_time(&meta), b)
        }
    };
    set_file_times_redox(f.as_raw_fd() as usize, atime1, mtime1)
}

fn open_redox(path: &Path, flags: i32) -> Result<Fd> {
    match path.to_str() {
        Some(string) => Fd::open(string, flags, 0),
        None => Err(Error::new(errno::EINVAL)),
    }
}

fn set_file_times_redox(fd: usize, atime: FileTime, mtime: FileTime) -> io::Result<()> {
    use libredox::data::TimeSpec;

    fn to_timespec(ft: &FileTime) -> TimeSpec {
        TimeSpec {
            tv_sec: ft.seconds(),
            tv_nsec: ft.nanoseconds() as _,
        }
    }

    let times = [to_timespec(&atime), to_timespec(&mtime)];

    call::futimens(fd, &times)?;
    Ok(())
}

pub fn from_last_modification_time(meta: &fs::Metadata) -> FileTime {
    FileTime {
        seconds: meta.mtime(),
        nanos: meta.mtime_nsec() as u32,
    }
}

pub fn from_last_access_time(meta: &fs::Metadata) -> FileTime {
    FileTime {
        seconds: meta.atime(),
        nanos: meta.atime_nsec() as u32,
    }
}

pub fn from_creation_time(_meta: &fs::Metadata) -> Option<FileTime> {
    None
}
