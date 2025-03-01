use alloc::borrow::Cow;
use alloc::sync::Arc;
use std::fs::File;
use std::path::PathBuf;

use object::Object;

use crate::{LookupContinuation, LookupResult};

#[cfg(unix)]
fn convert_path<R: gimli::Reader<Endian = gimli::RunTimeEndian>>(
    r: &R,
) -> Result<PathBuf, gimli::Error> {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let bytes = r.to_slice()?;
    let s = OsStr::from_bytes(&bytes);
    Ok(PathBuf::from(s))
}

#[cfg(not(unix))]
fn convert_path<R: gimli::Reader<Endian = gimli::RunTimeEndian>>(
    r: &R,
) -> Result<PathBuf, gimli::Error> {
    let bytes = r.to_slice()?;
    let s = std::str::from_utf8(&bytes).map_err(|_| gimli::Error::BadUtf8)?;
    Ok(PathBuf::from(s))
}

fn load_section<'data: 'file, 'file, O, R, F>(
    id: gimli::SectionId,
    file: &'file O,
    endian: R::Endian,
    loader: &mut F,
) -> Result<R, gimli::Error>
where
    O: object::Object<'data, 'file>,
    R: gimli::Reader<Endian = gimli::RunTimeEndian>,
    F: FnMut(Cow<'data, [u8]>, R::Endian) -> R,
{
    use object::ObjectSection;

    let data = id
        .dwo_name()
        .and_then(|dwo_name| {
            file.section_by_name(dwo_name)
                .and_then(|section| section.uncompressed_data().ok())
        })
        .unwrap_or(Cow::Borrowed(&[]));
    Ok(loader(data, endian))
}

/// A simple builtin split DWARF loader.
pub struct SplitDwarfLoader<R, F>
where
    R: gimli::Reader<Endian = gimli::RunTimeEndian>,
    F: FnMut(Cow<'_, [u8]>, R::Endian) -> R,
{
    loader: F,
    dwarf_package: Option<gimli::DwarfPackage<R>>,
}

impl<R, F> SplitDwarfLoader<R, F>
where
    R: gimli::Reader<Endian = gimli::RunTimeEndian>,
    F: FnMut(Cow<'_, [u8]>, R::Endian) -> R,
{
    fn load_dwarf_package(loader: &mut F, path: Option<PathBuf>) -> Option<gimli::DwarfPackage<R>> {
        let mut path = path.map(Ok).unwrap_or_else(std::env::current_exe).ok()?;
        let dwp_extension = path
            .extension()
            .map(|previous_extension| {
                let mut previous_extension = previous_extension.to_os_string();
                previous_extension.push(".dwp");
                previous_extension
            })
            .unwrap_or_else(|| "dwp".into());
        path.set_extension(dwp_extension);
        let file = File::open(&path).ok()?;
        let map = unsafe { memmap2::Mmap::map(&file).ok()? };
        let dwp = object::File::parse(&*map).ok()?;

        let endian = if dwp.is_little_endian() {
            gimli::RunTimeEndian::Little
        } else {
            gimli::RunTimeEndian::Big
        };

        let empty = loader(Cow::Borrowed(&[]), endian);
        gimli::DwarfPackage::load(
            |section_id| load_section(section_id, &dwp, endian, loader),
            empty,
        )
        .ok()
    }

    /// Create a new split DWARF loader.
    pub fn new(mut loader: F, path: Option<PathBuf>) -> SplitDwarfLoader<R, F> {
        let dwarf_package = SplitDwarfLoader::load_dwarf_package(&mut loader, path);
        SplitDwarfLoader {
            loader,
            dwarf_package,
        }
    }

    /// Run the provided `LookupResult` to completion, loading any necessary
    /// split DWARF along the way.
    pub fn run<L>(&mut self, mut l: LookupResult<L>) -> L::Output
    where
        L: LookupContinuation<Buf = R>,
    {
        loop {
            let (load, continuation) = match l {
                LookupResult::Output(output) => break output,
                LookupResult::Load { load, continuation } => (load, continuation),
            };

            let mut r: Option<Arc<gimli::Dwarf<_>>> = None;
            if let Some(dwp) = self.dwarf_package.as_ref() {
                if let Ok(Some(cu)) = dwp.find_cu(load.dwo_id, &load.parent) {
                    r = Some(Arc::new(cu));
                }
            }

            if r.is_none() {
                let mut path = PathBuf::new();
                if let Some(p) = load.comp_dir.as_ref() {
                    if let Ok(p) = convert_path(p) {
                        path.push(p);
                    }
                }

                if let Some(p) = load.path.as_ref() {
                    if let Ok(p) = convert_path(p) {
                        path.push(p);
                    }
                }

                if let Ok(file) = File::open(&path) {
                    if let Ok(map) = unsafe { memmap2::Mmap::map(&file) } {
                        if let Ok(file) = object::File::parse(&*map) {
                            let endian = if file.is_little_endian() {
                                gimli::RunTimeEndian::Little
                            } else {
                                gimli::RunTimeEndian::Big
                            };

                            r = gimli::Dwarf::load(|id| {
                                load_section(id, &file, endian, &mut self.loader)
                            })
                            .ok()
                            .map(|mut dwo_dwarf| {
                                dwo_dwarf.make_dwo(&load.parent);
                                Arc::new(dwo_dwarf)
                            });
                        }
                    }
                }
            }

            l = continuation.resume(r);
        }
    }
}
