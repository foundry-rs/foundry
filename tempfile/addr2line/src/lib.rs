//! This crate provides a cross-platform library and binary for translating addresses into
//! function names, file names and line numbers. Given an address in an executable or an
//! offset in a section of a relocatable object, it uses the debugging information to
//! figure out which file name and line number are associated with it.
//!
//! When used as a library, files must first be loaded using the
//! [`object`](https://github.com/gimli-rs/object) crate.
//! A context can then be created with [`Context::new`](./struct.Context.html#method.new).
//! The context caches some of the parsed information so that multiple lookups are
//! efficient.
//! Location information is obtained with
//! [`Context::find_location`](./struct.Context.html#method.find_location) or
//! [`Context::find_location_range`](./struct.Context.html#method.find_location_range).
//! Function information is obtained with
//! [`Context::find_frames`](./struct.Context.html#method.find_frames), which returns
//! a frame for each inline function. Each frame contains both name and location.
//!
//! The crate has an example CLI wrapper around the library which provides some of
//! the functionality of the `addr2line` command line tool distributed with [GNU
//! binutils](https://www.gnu.org/software/binutils/).
//!
//! Currently this library only provides information from the DWARF debugging information,
//! which is parsed using [`gimli`](https://github.com/gimli-rs/gimli).  The example CLI
//! wrapper also uses symbol table information provided by the `object` crate.
#![deny(missing_docs)]
#![no_std]

#[cfg(feature = "std")]
extern crate std;

#[allow(unused_imports)]
#[macro_use]
extern crate alloc;

#[cfg(feature = "fallible-iterator")]
pub extern crate fallible_iterator;
pub extern crate gimli;
#[cfg(feature = "object")]
pub extern crate object;

use alloc::borrow::Cow;
use alloc::boxed::Box;
#[cfg(feature = "object")]
use alloc::rc::Rc;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use core::cmp::{self, Ordering};
use core::iter;
use core::marker::PhantomData;
use core::mem;
use core::num::NonZeroU64;
use core::ops::ControlFlow;
use core::u64;

use crate::function::{Function, Functions, InlinedFunction};
use crate::lazy::LazyCell;

#[cfg(feature = "smallvec")]
mod maybe_small {
    pub type Vec<T> = smallvec::SmallVec<[T; 16]>;
    pub type IntoIter<T> = smallvec::IntoIter<[T; 16]>;
}
#[cfg(not(feature = "smallvec"))]
mod maybe_small {
    pub type Vec<T> = alloc::vec::Vec<T>;
    pub type IntoIter<T> = alloc::vec::IntoIter<T>;
}

#[cfg(all(feature = "std", feature = "object", feature = "memmap2"))]
/// A simple builtin split DWARF loader.
pub mod builtin_split_dwarf_loader;
mod function;
mod lazy;

type Error = gimli::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DebugFile {
    Primary,
    Supplementary,
    Dwo,
}

/// Operations that consult debug information may require additional files
/// to be loaded if split DWARF is being used. This enum returns the result
/// of the operation in the `Break` variant, or information about the split
/// DWARF that is required and a continuation to invoke once it is available
/// in the `Continue` variant.
///
/// This enum is intended to be used in a loop like so:
/// ```no_run
///   # use addr2line::*;
///   # use std::sync::Arc;
///   # let ctx: Context<gimli::EndianRcSlice<gimli::RunTimeEndian>> = todo!();
///   # let do_split_dwarf_load = |load: SplitDwarfLoad<gimli::EndianRcSlice<gimli::RunTimeEndian>>| -> Option<Arc<gimli::Dwarf<gimli::EndianRcSlice<gimli::RunTimeEndian>>>> { None };
///   const ADDRESS: u64 = 0xdeadbeef;
///   let mut r = ctx.find_frames(ADDRESS);
///   let result = loop {
///     match r {
///       LookupResult::Output(result) => break result,
///       LookupResult::Load { load, continuation } => {
///         let dwo = do_split_dwarf_load(load);
///         r = continuation.resume(dwo);
///       }
///     }
///   };
/// ```
pub enum LookupResult<L: LookupContinuation> {
    /// The lookup requires split DWARF data to be loaded.
    Load {
        /// The information needed to find the split DWARF data.
        load: SplitDwarfLoad<<L as LookupContinuation>::Buf>,
        /// The continuation to resume with the loaded split DWARF data.
        continuation: L,
    },
    /// The lookup has completed and produced an output.
    Output(<L as LookupContinuation>::Output),
}

/// This trait represents a partially complete operation that can be resumed
/// once a load of needed split DWARF data is completed or abandoned by the
/// API consumer.
pub trait LookupContinuation: Sized {
    /// The final output of this operation.
    type Output;
    /// The type of reader used.
    type Buf: gimli::Reader;

    /// Resumes the operation with the provided data.
    ///
    /// After the caller loads the split DWARF data required, call this
    /// method to resume the operation. The return value of this method
    /// indicates if the computation has completed or if further data is
    /// required.
    ///
    /// If the additional data cannot be located, or the caller does not
    /// support split DWARF, `resume(None)` can be used to continue the
    /// operation with the data that is available.
    fn resume(self, input: Option<Arc<gimli::Dwarf<Self::Buf>>>) -> LookupResult<Self>;
}

impl<L: LookupContinuation> LookupResult<L> {
    /// Callers that do not handle split DWARF can call `skip_all_loads`
    /// to fast-forward to the end result. This result is produced with
    /// the data that is available and may be less accurate than the
    /// the results that would be produced if the caller did properly
    /// support split DWARF.
    pub fn skip_all_loads(mut self) -> L::Output {
        loop {
            self = match self {
                LookupResult::Output(t) => return t,
                LookupResult::Load { continuation, .. } => continuation.resume(None),
            };
        }
    }

    fn map<T, F: FnOnce(L::Output) -> T>(self, f: F) -> LookupResult<MappedLookup<T, L, F>> {
        match self {
            LookupResult::Output(t) => LookupResult::Output(f(t)),
            LookupResult::Load { load, continuation } => LookupResult::Load {
                load,
                continuation: MappedLookup {
                    original: continuation,
                    mutator: f,
                },
            },
        }
    }

    fn unwrap(self) -> L::Output {
        match self {
            LookupResult::Output(t) => t,
            LookupResult::Load { .. } => unreachable!("Internal API misuse"),
        }
    }
}

/// The state necessary to perform address to line translation.
///
/// Constructing a `Context` is somewhat costly, so users should aim to reuse `Context`s
/// when performing lookups for many addresses in the same executable.
pub struct Context<R: gimli::Reader> {
    sections: Arc<gimli::Dwarf<R>>,
    unit_ranges: Box<[UnitRange]>,
    units: Box<[ResUnit<R>]>,
    sup_units: Box<[SupUnit<R>]>,
}

/// The type of `Context` that supports the `new` method.
#[cfg(feature = "std-object")]
pub type ObjectContext = Context<gimli::EndianRcSlice<gimli::RunTimeEndian>>;

#[cfg(feature = "std-object")]
impl Context<gimli::EndianRcSlice<gimli::RunTimeEndian>> {
    /// Construct a new `Context`.
    ///
    /// The resulting `Context` uses `gimli::EndianRcSlice<gimli::RunTimeEndian>`.
    /// This means it is not thread safe, has no lifetime constraints (since it copies
    /// the input data), and works for any endianity.
    ///
    /// Performance sensitive applications may want to use `Context::from_dwarf`
    /// with a more specialised `gimli::Reader` implementation.
    #[inline]
    pub fn new<'data: 'file, 'file, O: object::Object<'data, 'file>>(
        file: &'file O,
    ) -> Result<Self, Error> {
        Self::new_with_sup(file, None)
    }

    /// Construct a new `Context`.
    ///
    /// Optionally also use a supplementary object file.
    ///
    /// The resulting `Context` uses `gimli::EndianRcSlice<gimli::RunTimeEndian>`.
    /// This means it is not thread safe, has no lifetime constraints (since it copies
    /// the input data), and works for any endianity.
    ///
    /// Performance sensitive applications may want to use `Context::from_dwarf`
    /// with a more specialised `gimli::Reader` implementation.
    pub fn new_with_sup<'data: 'file, 'file, O: object::Object<'data, 'file>>(
        file: &'file O,
        sup_file: Option<&'file O>,
    ) -> Result<Self, Error> {
        let endian = if file.is_little_endian() {
            gimli::RunTimeEndian::Little
        } else {
            gimli::RunTimeEndian::Big
        };

        fn load_section<'data: 'file, 'file, O, Endian>(
            id: gimli::SectionId,
            file: &'file O,
            endian: Endian,
        ) -> Result<gimli::EndianRcSlice<Endian>, Error>
        where
            O: object::Object<'data, 'file>,
            Endian: gimli::Endianity,
        {
            use object::ObjectSection;

            let data = file
                .section_by_name(id.name())
                .and_then(|section| section.uncompressed_data().ok())
                .unwrap_or(Cow::Borrowed(&[]));
            Ok(gimli::EndianRcSlice::new(Rc::from(&*data), endian))
        }

        let mut dwarf = gimli::Dwarf::load(|id| load_section(id, file, endian))?;
        if let Some(sup_file) = sup_file {
            dwarf.load_sup(|id| load_section(id, sup_file, endian))?;
        }
        Context::from_dwarf(dwarf)
    }
}

impl<R: gimli::Reader> Context<R> {
    /// Construct a new `Context` from DWARF sections.
    ///
    /// This method does not support using a supplementary object file.
    pub fn from_sections(
        debug_abbrev: gimli::DebugAbbrev<R>,
        debug_addr: gimli::DebugAddr<R>,
        debug_aranges: gimli::DebugAranges<R>,
        debug_info: gimli::DebugInfo<R>,
        debug_line: gimli::DebugLine<R>,
        debug_line_str: gimli::DebugLineStr<R>,
        debug_ranges: gimli::DebugRanges<R>,
        debug_rnglists: gimli::DebugRngLists<R>,
        debug_str: gimli::DebugStr<R>,
        debug_str_offsets: gimli::DebugStrOffsets<R>,
        default_section: R,
    ) -> Result<Self, Error> {
        Self::from_dwarf(gimli::Dwarf {
            debug_abbrev,
            debug_addr,
            debug_aranges,
            debug_info,
            debug_line,
            debug_line_str,
            debug_str,
            debug_str_offsets,
            debug_types: default_section.clone().into(),
            locations: gimli::LocationLists::new(
                default_section.clone().into(),
                default_section.into(),
            ),
            ranges: gimli::RangeLists::new(debug_ranges, debug_rnglists),
            file_type: gimli::DwarfFileType::Main,
            sup: None,
            abbreviations_cache: gimli::AbbreviationsCache::new(),
        })
    }

    /// Construct a new `Context` from an existing [`gimli::Dwarf`] object.
    #[inline]
    pub fn from_dwarf(sections: gimli::Dwarf<R>) -> Result<Context<R>, Error> {
        let sections = Arc::new(sections);
        let (unit_ranges, units) = Context::parse_units(&sections)?;
        let sup_units = if let Some(sup) = sections.sup.as_ref() {
            Context::parse_sup(sup)?
        } else {
            Vec::new()
        };
        Ok(Context {
            sections,
            unit_ranges: unit_ranges.into_boxed_slice(),
            units: units.into_boxed_slice(),
            sup_units: sup_units.into_boxed_slice(),
        })
    }

    /// Finds the CUs for the function address given.
    ///
    /// There might be multiple CUs whose range contains this address.
    /// Weak symbols have shown up in the wild which cause this to happen
    /// but otherwise this can happen if the CU has non-contiguous functions
    /// but only reports a single range.
    ///
    /// Consequently we return an iterator for all CUs which may contain the
    /// address, and the caller must check if there is actually a function or
    /// location in the CU for that address.
    fn find_units(&self, probe: u64) -> impl Iterator<Item = &ResUnit<R>> {
        self.find_units_range(probe, probe + 1)
            .map(|(unit, _range)| unit)
    }

    /// Finds the CUs covering the range of addresses given.
    ///
    /// The range is [low, high) (ie, the upper bound is exclusive). This can return multiple
    /// ranges for the same unit.
    #[inline]
    fn find_units_range(
        &self,
        probe_low: u64,
        probe_high: u64,
    ) -> impl Iterator<Item = (&ResUnit<R>, &gimli::Range)> {
        // First up find the position in the array which could have our function
        // address.
        let pos = match self
            .unit_ranges
            .binary_search_by_key(&probe_high, |i| i.range.begin)
        {
            // Although unlikely, we could find an exact match.
            Ok(i) => i + 1,
            // No exact match was found, but this probe would fit at slot `i`.
            // This means that slot `i` is bigger than `probe`, along with all
            // indices greater than `i`, so we need to search all previous
            // entries.
            Err(i) => i,
        };

        // Once we have our index we iterate backwards from that position
        // looking for a matching CU.
        self.unit_ranges[..pos]
            .iter()
            .rev()
            .take_while(move |i| {
                // We know that this CU's start is beneath the probe already because
                // of our sorted array.
                debug_assert!(i.range.begin <= probe_high);

                // Each entry keeps track of the maximum end address seen so far,
                // starting from the beginning of the array of unit ranges. We're
                // iterating in reverse so if our probe is beyond the maximum range
                // of this entry, then it's guaranteed to not fit in any prior
                // entries, so we break out.
                probe_low < i.max_end
            })
            .filter_map(move |i| {
                // If this CU doesn't actually contain this address, move to the
                // next CU.
                if probe_low >= i.range.end || probe_high <= i.range.begin {
                    return None;
                }
                Some((&self.units[i.unit_id], &i.range))
            })
    }

    /// Find the DWARF unit corresponding to the given virtual memory address.
    pub fn find_dwarf_and_unit(
        &self,
        probe: u64,
    ) -> LookupResult<
        impl LookupContinuation<Output = Option<(&gimli::Dwarf<R>, &gimli::Unit<R>)>, Buf = R>,
    > {
        let mut units_iter = self.find_units(probe);
        if let Some(unit) = units_iter.next() {
            return LoopingLookup::new_lookup(
                unit.find_function_or_location(probe, self),
                move |r| {
                    ControlFlow::Break(match r {
                        Ok((Some(_), _)) | Ok((_, Some(_))) => {
                            let (_file, sections, unit) = unit
                                .dwarf_and_unit_dwo(self)
                                // We've already been through both error cases here to get to this point.
                                .unwrap()
                                .unwrap();
                            Some((sections, unit))
                        }
                        _ => match units_iter.next() {
                            Some(next_unit) => {
                                return ControlFlow::Continue(
                                    next_unit.find_function_or_location(probe, self),
                                );
                            }
                            None => None,
                        },
                    })
                },
            );
        }

        LoopingLookup::new_complete(None)
    }

    /// Find the source file and line corresponding to the given virtual memory address.
    pub fn find_location(&self, probe: u64) -> Result<Option<Location<'_>>, Error> {
        for unit in self.find_units(probe) {
            if let Some(location) = unit.find_location(probe, &self.sections)? {
                return Ok(Some(location));
            }
        }
        Ok(None)
    }

    /// Return source file and lines for a range of addresses. For each location it also
    /// returns the address and size of the range of the underlying instructions.
    pub fn find_location_range(
        &self,
        probe_low: u64,
        probe_high: u64,
    ) -> Result<LocationRangeIter<'_, R>, Error> {
        LocationRangeIter::new(self, probe_low, probe_high)
    }

    /// Return an iterator for the function frames corresponding to the given virtual
    /// memory address.
    ///
    /// If the probe address is not for an inline function then only one frame is
    /// returned.
    ///
    /// If the probe address is for an inline function then the first frame corresponds
    /// to the innermost inline function.  Subsequent frames contain the caller and call
    /// location, until an non-inline caller is reached.
    pub fn find_frames(
        &self,
        probe: u64,
    ) -> LookupResult<impl LookupContinuation<Output = Result<FrameIter<'_, R>, Error>, Buf = R>>
    {
        let mut units_iter = self.find_units(probe);
        if let Some(unit) = units_iter.next() {
            LoopingLookup::new_lookup(unit.find_function_or_location(probe, self), move |r| {
                ControlFlow::Break(match r {
                    Err(e) => Err(e),
                    Ok((Some(function), location)) => {
                        let inlined_functions = function.find_inlined_functions(probe);
                        Ok(FrameIter(FrameIterState::Frames(FrameIterFrames {
                            unit,
                            sections: &self.sections,
                            function,
                            inlined_functions,
                            next: location,
                        })))
                    }
                    Ok((None, Some(location))) => {
                        Ok(FrameIter(FrameIterState::Location(Some(location))))
                    }
                    Ok((None, None)) => match units_iter.next() {
                        Some(next_unit) => {
                            return ControlFlow::Continue(
                                next_unit.find_function_or_location(probe, self),
                            );
                        }
                        None => Ok(FrameIter(FrameIterState::Empty)),
                    },
                })
            })
        } else {
            LoopingLookup::new_complete(Ok(FrameIter(FrameIterState::Empty)))
        }
    }

    /// Preload units for `probe`.
    ///
    /// The iterator returns pairs of `SplitDwarfLoad`s containing the
    /// information needed to locate and load split DWARF for `probe` and
    /// a matching callback to invoke once that data is available.
    ///
    /// If this method is called, and all of the returned closures are invoked,
    /// addr2line guarantees that any future API call for the address `probe`
    /// will not require the loading of any split DWARF.
    ///
    /// ```no_run
    ///   # use addr2line::*;
    ///   # use std::sync::Arc;
    ///   # let ctx: Context<gimli::EndianRcSlice<gimli::RunTimeEndian>> = todo!();
    ///   # let do_split_dwarf_load = |load: SplitDwarfLoad<gimli::EndianRcSlice<gimli::RunTimeEndian>>| -> Option<Arc<gimli::Dwarf<gimli::EndianRcSlice<gimli::RunTimeEndian>>>> { None };
    ///   const ADDRESS: u64 = 0xdeadbeef;
    ///   ctx.preload_units(ADDRESS).for_each(|(load, callback)| {
    ///     let dwo = do_split_dwarf_load(load);
    ///     callback(dwo);
    ///   });
    ///
    ///   let frames_iter = match ctx.find_frames(ADDRESS) {
    ///     LookupResult::Output(result) => result,
    ///     LookupResult::Load { .. } => unreachable!("addr2line promised we wouldn't get here"),
    ///   };
    ///
    ///   // ...
    /// ```
    pub fn preload_units(
        &'_ self,
        probe: u64,
    ) -> impl Iterator<
        Item = (
            SplitDwarfLoad<R>,
            impl FnOnce(Option<Arc<gimli::Dwarf<R>>>) -> Result<(), gimli::Error> + '_,
        ),
    > {
        self.find_units(probe)
            .filter_map(move |unit| match unit.dwarf_and_unit_dwo(self) {
                LookupResult::Output(_) => None,
                LookupResult::Load { load, continuation } => Some((load, |result| {
                    continuation.resume(result).unwrap().map(|_| ())
                })),
            })
    }

    /// Initialize all line data structures. This is used for benchmarks.
    #[doc(hidden)]
    pub fn parse_lines(&self) -> Result<(), Error> {
        for unit in self.units.iter() {
            unit.parse_lines(&self.sections)?;
        }
        Ok(())
    }

    /// Initialize all function data structures. This is used for benchmarks.
    #[doc(hidden)]
    pub fn parse_functions(&self) -> Result<(), Error> {
        for unit in self.units.iter() {
            unit.parse_functions(self).skip_all_loads()?;
        }
        Ok(())
    }

    /// Initialize all inlined function data structures. This is used for benchmarks.
    #[doc(hidden)]
    pub fn parse_inlined_functions(&self) -> Result<(), Error> {
        for unit in self.units.iter() {
            unit.parse_inlined_functions(self).skip_all_loads()?;
        }
        Ok(())
    }
}

struct UnitRange {
    unit_id: usize,
    max_end: u64,
    range: gimli::Range,
}

struct ResUnit<R: gimli::Reader> {
    offset: gimli::DebugInfoOffset<R::Offset>,
    dw_unit: gimli::Unit<R>,
    lang: Option<gimli::DwLang>,
    lines: LazyCell<Result<Lines, Error>>,
    funcs: LazyCell<Result<Functions<R>, Error>>,
    dwo: LazyCell<Result<Option<Box<(Arc<gimli::Dwarf<R>>, gimli::Unit<R>)>>, Error>>,
}

struct SupUnit<R: gimli::Reader> {
    offset: gimli::DebugInfoOffset<R::Offset>,
    dw_unit: gimli::Unit<R>,
}

impl<R: gimli::Reader> Context<R> {
    fn parse_units(sections: &gimli::Dwarf<R>) -> Result<(Vec<UnitRange>, Vec<ResUnit<R>>), Error> {
        // Find all the references to compilation units in .debug_aranges.
        // Note that we always also iterate through all of .debug_info to
        // find compilation units, because .debug_aranges may be missing some.
        let mut aranges = Vec::new();
        let mut headers = sections.debug_aranges.headers();
        while let Some(header) = headers.next()? {
            aranges.push((header.debug_info_offset(), header.offset()));
        }
        aranges.sort_by_key(|i| i.0);

        let mut unit_ranges = Vec::new();
        let mut res_units = Vec::new();
        let mut units = sections.units();
        while let Some(header) = units.next()? {
            let unit_id = res_units.len();
            let offset = match header.offset().as_debug_info_offset() {
                Some(offset) => offset,
                None => continue,
            };
            // We mainly want compile units, but we may need to follow references to entries
            // within other units for function names.  We don't need anything from type units.
            match header.type_() {
                gimli::UnitType::Type { .. } | gimli::UnitType::SplitType { .. } => continue,
                _ => {}
            }
            let dw_unit = match sections.unit(header) {
                Ok(dw_unit) => dw_unit,
                Err(_) => continue,
            };

            let mut lang = None;
            let mut have_unit_range = false;
            {
                let mut entries = dw_unit.entries_raw(None)?;

                let abbrev = match entries.read_abbreviation()? {
                    Some(abbrev) => abbrev,
                    None => continue,
                };

                let mut ranges = RangeAttributes::default();
                for spec in abbrev.attributes() {
                    let attr = entries.read_attribute(*spec)?;
                    match attr.name() {
                        gimli::DW_AT_low_pc => match attr.value() {
                            gimli::AttributeValue::Addr(val) => ranges.low_pc = Some(val),
                            gimli::AttributeValue::DebugAddrIndex(index) => {
                                ranges.low_pc = Some(sections.address(&dw_unit, index)?);
                            }
                            _ => {}
                        },
                        gimli::DW_AT_high_pc => match attr.value() {
                            gimli::AttributeValue::Addr(val) => ranges.high_pc = Some(val),
                            gimli::AttributeValue::DebugAddrIndex(index) => {
                                ranges.high_pc = Some(sections.address(&dw_unit, index)?);
                            }
                            gimli::AttributeValue::Udata(val) => ranges.size = Some(val),
                            _ => {}
                        },
                        gimli::DW_AT_ranges => {
                            ranges.ranges_offset =
                                sections.attr_ranges_offset(&dw_unit, attr.value())?;
                        }
                        gimli::DW_AT_language => {
                            if let gimli::AttributeValue::Language(val) = attr.value() {
                                lang = Some(val);
                            }
                        }
                        _ => {}
                    }
                }

                // Find the address ranges for the CU, using in order of preference:
                // - DW_AT_ranges
                // - .debug_aranges
                // - DW_AT_low_pc/DW_AT_high_pc
                //
                // Using DW_AT_ranges before .debug_aranges is possibly an arbitrary choice,
                // but the feeling is that DW_AT_ranges is more likely to be reliable or complete
                // if it is present.
                //
                // .debug_aranges must be used before DW_AT_low_pc/DW_AT_high_pc because
                // it has been observed on macOS that DW_AT_ranges was not emitted even for
                // discontiguous CUs.
                let i = match ranges.ranges_offset {
                    Some(_) => None,
                    None => aranges.binary_search_by_key(&offset, |x| x.0).ok(),
                };
                if let Some(mut i) = i {
                    // There should be only one set per CU, but in practice multiple
                    // sets have been observed. This is probably a compiler bug, but
                    // either way we need to handle it.
                    while i > 0 && aranges[i - 1].0 == offset {
                        i -= 1;
                    }
                    for (_, aranges_offset) in aranges[i..].iter().take_while(|x| x.0 == offset) {
                        let aranges_header = sections.debug_aranges.header(*aranges_offset)?;
                        let mut aranges = aranges_header.entries();
                        while let Some(arange) = aranges.next()? {
                            if arange.length() != 0 {
                                unit_ranges.push(UnitRange {
                                    range: arange.range(),
                                    unit_id,
                                    max_end: 0,
                                });
                                have_unit_range = true;
                            }
                        }
                    }
                } else {
                    have_unit_range |= ranges.for_each_range(sections, &dw_unit, |range| {
                        unit_ranges.push(UnitRange {
                            range,
                            unit_id,
                            max_end: 0,
                        });
                    })?;
                }
            }

            let lines = LazyCell::new();
            if !have_unit_range {
                // The unit did not declare any ranges.
                // Try to get some ranges from the line program sequences.
                if let Some(ref ilnp) = dw_unit.line_program {
                    if let Ok(lines) = lines
                        .borrow_with(|| Lines::parse(&dw_unit, ilnp.clone(), sections))
                        .as_ref()
                    {
                        for sequence in lines.sequences.iter() {
                            unit_ranges.push(UnitRange {
                                range: gimli::Range {
                                    begin: sequence.start,
                                    end: sequence.end,
                                },
                                unit_id,
                                max_end: 0,
                            })
                        }
                    }
                }
            }

            res_units.push(ResUnit {
                offset,
                dw_unit,
                lang,
                lines,
                funcs: LazyCell::new(),
                dwo: LazyCell::new(),
            });
        }

        // Sort this for faster lookup in `find_unit_and_address` below.
        unit_ranges.sort_by_key(|i| i.range.begin);

        // Calculate the `max_end` field now that we've determined the order of
        // CUs.
        let mut max = 0;
        for i in unit_ranges.iter_mut() {
            max = max.max(i.range.end);
            i.max_end = max;
        }

        Ok((unit_ranges, res_units))
    }

    fn parse_sup(sections: &gimli::Dwarf<R>) -> Result<Vec<SupUnit<R>>, Error> {
        let mut sup_units = Vec::new();
        let mut units = sections.units();
        while let Some(header) = units.next()? {
            let offset = match header.offset().as_debug_info_offset() {
                Some(offset) => offset,
                None => continue,
            };
            let dw_unit = match sections.unit(header) {
                Ok(dw_unit) => dw_unit,
                Err(_) => continue,
            };
            sup_units.push(SupUnit { dw_unit, offset });
        }
        Ok(sup_units)
    }

    // Find the unit containing the given offset, and convert the offset into a unit offset.
    fn find_unit(
        &self,
        offset: gimli::DebugInfoOffset<R::Offset>,
        file: DebugFile,
    ) -> Result<(&gimli::Unit<R>, gimli::UnitOffset<R::Offset>), Error> {
        let unit = match file {
            DebugFile::Primary => {
                match self
                    .units
                    .binary_search_by_key(&offset.0, |unit| unit.offset.0)
                {
                    // There is never a DIE at the unit offset or before the first unit.
                    Ok(_) | Err(0) => return Err(gimli::Error::NoEntryAtGivenOffset),
                    Err(i) => &self.units[i - 1].dw_unit,
                }
            }
            DebugFile::Supplementary => {
                match self
                    .sup_units
                    .binary_search_by_key(&offset.0, |unit| unit.offset.0)
                {
                    // There is never a DIE at the unit offset or before the first unit.
                    Ok(_) | Err(0) => return Err(gimli::Error::NoEntryAtGivenOffset),
                    Err(i) => &self.sup_units[i - 1].dw_unit,
                }
            }
            DebugFile::Dwo => return Err(gimli::Error::NoEntryAtGivenOffset),
        };

        let unit_offset = offset
            .to_unit_offset(&unit.header)
            .ok_or(gimli::Error::NoEntryAtGivenOffset)?;
        Ok((unit, unit_offset))
    }
}

struct Lines {
    files: Box<[String]>,
    sequences: Box<[LineSequence]>,
}

impl Lines {
    fn parse<R: gimli::Reader>(
        dw_unit: &gimli::Unit<R>,
        ilnp: gimli::IncompleteLineProgram<R, R::Offset>,
        sections: &gimli::Dwarf<R>,
    ) -> Result<Self, Error> {
        let mut sequences = Vec::new();
        let mut sequence_rows = Vec::<LineRow>::new();
        let mut rows = ilnp.rows();
        while let Some((_, row)) = rows.next_row()? {
            if row.end_sequence() {
                if let Some(start) = sequence_rows.first().map(|x| x.address) {
                    let end = row.address();
                    let mut rows = Vec::new();
                    mem::swap(&mut rows, &mut sequence_rows);
                    sequences.push(LineSequence {
                        start,
                        end,
                        rows: rows.into_boxed_slice(),
                    });
                }
                continue;
            }

            let address = row.address();
            let file_index = row.file_index();
            let line = row.line().map(NonZeroU64::get).unwrap_or(0) as u32;
            let column = match row.column() {
                gimli::ColumnType::LeftEdge => 0,
                gimli::ColumnType::Column(x) => x.get() as u32,
            };

            if let Some(last_row) = sequence_rows.last_mut() {
                if last_row.address == address {
                    last_row.file_index = file_index;
                    last_row.line = line;
                    last_row.column = column;
                    continue;
                }
            }

            sequence_rows.push(LineRow {
                address,
                file_index,
                line,
                column,
            });
        }
        sequences.sort_by_key(|x| x.start);

        let mut files = Vec::new();
        let header = rows.header();
        match header.file(0) {
            Some(file) => files.push(render_file(dw_unit, file, header, sections)?),
            None => files.push(String::from("")), // DWARF version <= 4 may not have 0th index
        }
        let mut index = 1;
        while let Some(file) = header.file(index) {
            files.push(render_file(dw_unit, file, header, sections)?);
            index += 1;
        }

        Ok(Self {
            files: files.into_boxed_slice(),
            sequences: sequences.into_boxed_slice(),
        })
    }
}

fn render_file<R: gimli::Reader>(
    dw_unit: &gimli::Unit<R>,
    file: &gimli::FileEntry<R, R::Offset>,
    header: &gimli::LineProgramHeader<R, R::Offset>,
    sections: &gimli::Dwarf<R>,
) -> Result<String, gimli::Error> {
    let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
        comp_dir.to_string_lossy()?.into_owned()
    } else {
        String::new()
    };

    // The directory index 0 is defined to correspond to the compilation unit directory.
    if file.directory_index() != 0 {
        if let Some(directory) = file.directory(header) {
            path_push(
                &mut path,
                sections
                    .attr_string(dw_unit, directory)?
                    .to_string_lossy()?
                    .as_ref(),
            );
        }
    }

    path_push(
        &mut path,
        sections
            .attr_string(dw_unit, file.path_name())?
            .to_string_lossy()?
            .as_ref(),
    );

    Ok(path)
}

struct LineSequence {
    start: u64,
    end: u64,
    rows: Box<[LineRow]>,
}

struct LineRow {
    address: u64,
    file_index: u64,
    line: u32,
    column: u32,
}

/// This struct contains the information needed to find split DWARF data
/// and to produce a `gimli::Dwarf<R>` for it.
pub struct SplitDwarfLoad<R> {
    /// The dwo id, for looking up in a DWARF package, or for
    /// verifying an unpacked dwo found on the file system
    pub dwo_id: gimli::DwoId,
    /// The compilation directory `path` is relative to.
    pub comp_dir: Option<R>,
    /// A path on the filesystem, relative to `comp_dir` to find this dwo.
    pub path: Option<R>,
    /// Once the split DWARF data is loaded, the loader is expected
    /// to call [make_dwo(parent)](gimli::read::Dwarf::make_dwo) before
    /// returning the data.
    pub parent: Arc<gimli::Dwarf<R>>,
}

struct SimpleLookup<T, R, F>
where
    F: FnOnce(Option<Arc<gimli::Dwarf<R>>>) -> T,
    R: gimli::Reader,
{
    f: F,
    phantom: PhantomData<(T, R)>,
}

impl<T, R, F> SimpleLookup<T, R, F>
where
    F: FnOnce(Option<Arc<gimli::Dwarf<R>>>) -> T,
    R: gimli::Reader,
{
    fn new_complete(t: F::Output) -> LookupResult<SimpleLookup<T, R, F>> {
        LookupResult::Output(t)
    }

    fn new_needs_load(load: SplitDwarfLoad<R>, f: F) -> LookupResult<SimpleLookup<T, R, F>> {
        LookupResult::Load {
            load,
            continuation: SimpleLookup {
                f,
                phantom: PhantomData,
            },
        }
    }
}

impl<T, R, F> LookupContinuation for SimpleLookup<T, R, F>
where
    F: FnOnce(Option<Arc<gimli::Dwarf<R>>>) -> T,
    R: gimli::Reader,
{
    type Output = T;
    type Buf = R;

    fn resume(self, v: Option<Arc<gimli::Dwarf<Self::Buf>>>) -> LookupResult<Self> {
        LookupResult::Output((self.f)(v))
    }
}

struct MappedLookup<T, L, F>
where
    L: LookupContinuation,
    F: FnOnce(L::Output) -> T,
{
    original: L,
    mutator: F,
}

impl<T, L, F> LookupContinuation for MappedLookup<T, L, F>
where
    L: LookupContinuation,
    F: FnOnce(L::Output) -> T,
{
    type Output = T;
    type Buf = L::Buf;

    fn resume(self, v: Option<Arc<gimli::Dwarf<Self::Buf>>>) -> LookupResult<Self> {
        match self.original.resume(v) {
            LookupResult::Output(t) => LookupResult::Output((self.mutator)(t)),
            LookupResult::Load { load, continuation } => LookupResult::Load {
                load,
                continuation: MappedLookup {
                    original: continuation,
                    mutator: self.mutator,
                },
            },
        }
    }
}

/// Some functions (e.g. `find_frames`) require considering multiple
/// compilation units, each of which might require their own split DWARF
/// lookup (and thus produce a continuation).
///
/// We store the underlying continuation here as well as a mutator function
/// that will either a) decide that the result of this continuation is
/// what is needed and mutate it to the final result or b) produce another
/// `LookupResult`. `new_lookup` will in turn eagerly drive any non-continuation
/// `LookupResult` with successive invocations of the mutator, until a new
/// continuation or a final result is produced. And finally, the impl of
/// `LookupContinuation::resume` will call `new_lookup` each time the
/// computation is resumed.
struct LoopingLookup<T, L, F>
where
    L: LookupContinuation,
    F: FnMut(L::Output) -> ControlFlow<T, LookupResult<L>>,
{
    continuation: L,
    mutator: F,
}

impl<T, L, F> LoopingLookup<T, L, F>
where
    L: LookupContinuation,
    F: FnMut(L::Output) -> ControlFlow<T, LookupResult<L>>,
{
    fn new_complete(t: T) -> LookupResult<Self> {
        LookupResult::Output(t)
    }

    fn new_lookup(mut r: LookupResult<L>, mut mutator: F) -> LookupResult<Self> {
        // Drive the loop eagerly so that we only ever have to represent one state
        // (the r == ControlFlow::Continue state) in LoopingLookup.
        loop {
            match r {
                LookupResult::Output(l) => match mutator(l) {
                    ControlFlow::Break(t) => return LookupResult::Output(t),
                    ControlFlow::Continue(r2) => {
                        r = r2;
                    }
                },
                LookupResult::Load { load, continuation } => {
                    return LookupResult::Load {
                        load,
                        continuation: LoopingLookup {
                            continuation,
                            mutator,
                        },
                    };
                }
            }
        }
    }
}

impl<T, L, F> LookupContinuation for LoopingLookup<T, L, F>
where
    L: LookupContinuation,
    F: FnMut(L::Output) -> ControlFlow<T, LookupResult<L>>,
{
    type Output = T;
    type Buf = L::Buf;

    fn resume(self, v: Option<Arc<gimli::Dwarf<Self::Buf>>>) -> LookupResult<Self> {
        let r = self.continuation.resume(v);
        LoopingLookup::new_lookup(r, self.mutator)
    }
}

impl<R: gimli::Reader> ResUnit<R> {
    fn dwarf_and_unit_dwo<'unit, 'ctx: 'unit>(
        &'unit self,
        ctx: &'ctx Context<R>,
    ) -> LookupResult<
        SimpleLookup<
            Result<(DebugFile, &'unit gimli::Dwarf<R>, &'unit gimli::Unit<R>), Error>,
            R,
            impl FnOnce(
                Option<Arc<gimli::Dwarf<R>>>,
            )
                -> Result<(DebugFile, &'unit gimli::Dwarf<R>, &'unit gimli::Unit<R>), Error>,
        >,
    > {
        loop {
            break SimpleLookup::new_complete(match self.dwo.borrow() {
                Some(Ok(Some(v))) => Ok((DebugFile::Dwo, &*v.0, &v.1)),
                Some(Ok(None)) => Ok((DebugFile::Primary, &*ctx.sections, &self.dw_unit)),
                Some(Err(e)) => Err(*e),
                None => {
                    let dwo_id = match self.dw_unit.dwo_id {
                        None => {
                            self.dwo.borrow_with(|| Ok(None));
                            continue;
                        }
                        Some(dwo_id) => dwo_id,
                    };

                    let comp_dir = self.dw_unit.comp_dir.clone();

                    let dwo_name = self.dw_unit.dwo_name().and_then(|s| {
                        if let Some(s) = s {
                            Ok(Some(ctx.sections.attr_string(&self.dw_unit, s)?))
                        } else {
                            Ok(None)
                        }
                    });

                    let path = match dwo_name {
                        Ok(v) => v,
                        Err(e) => {
                            self.dwo.borrow_with(|| Err(e));
                            continue;
                        }
                    };

                    let process_dwo = move |dwo_dwarf: Option<Arc<gimli::Dwarf<R>>>| {
                        let dwo_dwarf = match dwo_dwarf {
                            None => return Ok(None),
                            Some(dwo_dwarf) => dwo_dwarf,
                        };
                        let mut dwo_units = dwo_dwarf.units();
                        let dwo_header = match dwo_units.next()? {
                            Some(dwo_header) => dwo_header,
                            None => return Ok(None),
                        };

                        let mut dwo_unit = dwo_dwarf.unit(dwo_header)?;
                        dwo_unit.copy_relocated_attributes(&self.dw_unit);
                        Ok(Some(Box::new((dwo_dwarf, dwo_unit))))
                    };

                    return SimpleLookup::new_needs_load(
                        SplitDwarfLoad {
                            dwo_id,
                            comp_dir,
                            path,
                            parent: ctx.sections.clone(),
                        },
                        move |dwo_dwarf| match self.dwo.borrow_with(|| process_dwo(dwo_dwarf)) {
                            Ok(Some(v)) => Ok((DebugFile::Dwo, &*v.0, &v.1)),
                            Ok(None) => Ok((DebugFile::Primary, &*ctx.sections, &self.dw_unit)),
                            Err(e) => Err(*e),
                        },
                    );
                }
            });
        }
    }

    fn parse_lines(&self, sections: &gimli::Dwarf<R>) -> Result<Option<&Lines>, Error> {
        // NB: line information is always stored in the main debug file so this does not need
        // to handle DWOs.
        let ilnp = match self.dw_unit.line_program {
            Some(ref ilnp) => ilnp,
            None => return Ok(None),
        };
        self.lines
            .borrow_with(|| Lines::parse(&self.dw_unit, ilnp.clone(), sections))
            .as_ref()
            .map(Some)
            .map_err(Error::clone)
    }

    fn parse_functions_dwarf_and_unit(
        &self,
        unit: &gimli::Unit<R>,
        sections: &gimli::Dwarf<R>,
    ) -> Result<&Functions<R>, Error> {
        self.funcs
            .borrow_with(|| Functions::parse(unit, sections))
            .as_ref()
            .map_err(Error::clone)
    }

    fn parse_functions<'unit, 'ctx: 'unit>(
        &'unit self,
        ctx: &'ctx Context<R>,
    ) -> LookupResult<impl LookupContinuation<Output = Result<&'unit Functions<R>, Error>, Buf = R>>
    {
        self.dwarf_and_unit_dwo(ctx).map(move |r| {
            let (_file, sections, unit) = r?;
            self.parse_functions_dwarf_and_unit(unit, sections)
        })
    }
    fn parse_inlined_functions<'unit, 'ctx: 'unit>(
        &'unit self,
        ctx: &'ctx Context<R>,
    ) -> LookupResult<impl LookupContinuation<Output = Result<(), Error>, Buf = R> + 'unit> {
        self.dwarf_and_unit_dwo(ctx).map(move |r| {
            let (file, sections, unit) = r?;
            self.funcs
                .borrow_with(|| Functions::parse(unit, sections))
                .as_ref()
                .map_err(Error::clone)?
                .parse_inlined_functions(file, unit, ctx, sections)
        })
    }

    fn find_location(
        &self,
        probe: u64,
        sections: &gimli::Dwarf<R>,
    ) -> Result<Option<Location<'_>>, Error> {
        if let Some(mut iter) = LocationRangeUnitIter::new(self, sections, probe, probe + 1)? {
            match iter.next() {
                None => Ok(None),
                Some((_addr, _len, loc)) => Ok(Some(loc)),
            }
        } else {
            Ok(None)
        }
    }

    #[inline]
    fn find_location_range(
        &self,
        probe_low: u64,
        probe_high: u64,
        sections: &gimli::Dwarf<R>,
    ) -> Result<Option<LocationRangeUnitIter<'_>>, Error> {
        LocationRangeUnitIter::new(self, sections, probe_low, probe_high)
    }

    fn find_function_or_location<'unit, 'ctx: 'unit>(
        &'unit self,
        probe: u64,
        ctx: &'ctx Context<R>,
    ) -> LookupResult<
        impl LookupContinuation<
            Output = Result<(Option<&'unit Function<R>>, Option<Location<'unit>>), Error>,
            Buf = R,
        >,
    > {
        self.dwarf_and_unit_dwo(ctx).map(move |r| {
            let (file, sections, unit) = r?;
            let functions = self.parse_functions_dwarf_and_unit(unit, sections)?;
            let function = match functions.find_address(probe) {
                Some(address) => {
                    let function_index = functions.addresses[address].function;
                    let (offset, ref function) = functions.functions[function_index];
                    Some(
                        function
                            .borrow_with(|| Function::parse(offset, file, unit, ctx, sections))
                            .as_ref()
                            .map_err(Error::clone)?,
                    )
                }
                None => None,
            };
            let location = self.find_location(probe, sections)?;
            Ok((function, location))
        })
    }
}

/// Iterator over `Location`s in a range of addresses, returned by `Context::find_location_range`.
pub struct LocationRangeIter<'ctx, R: gimli::Reader> {
    unit_iter: Box<dyn Iterator<Item = (&'ctx ResUnit<R>, &'ctx gimli::Range)> + 'ctx>,
    iter: Option<LocationRangeUnitIter<'ctx>>,

    probe_low: u64,
    probe_high: u64,
    sections: &'ctx gimli::Dwarf<R>,
}

impl<'ctx, R: gimli::Reader> LocationRangeIter<'ctx, R> {
    #[inline]
    fn new(ctx: &'ctx Context<R>, probe_low: u64, probe_high: u64) -> Result<Self, Error> {
        let sections = &ctx.sections;
        let unit_iter = ctx.find_units_range(probe_low, probe_high);
        Ok(Self {
            unit_iter: Box::new(unit_iter),
            iter: None,
            probe_low,
            probe_high,
            sections,
        })
    }

    fn next_loc(&mut self) -> Result<Option<(u64, u64, Location<'ctx>)>, Error> {
        loop {
            let iter = self.iter.take();
            match iter {
                None => match self.unit_iter.next() {
                    Some((unit, range)) => {
                        self.iter = unit.find_location_range(
                            cmp::max(self.probe_low, range.begin),
                            cmp::min(self.probe_high, range.end),
                            self.sections,
                        )?;
                    }
                    None => return Ok(None),
                },
                Some(mut iter) => {
                    if let item @ Some(_) = iter.next() {
                        self.iter = Some(iter);
                        return Ok(item);
                    }
                }
            }
        }
    }
}

impl<'ctx, R> Iterator for LocationRangeIter<'ctx, R>
where
    R: gimli::Reader + 'ctx,
{
    type Item = (u64, u64, Location<'ctx>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        match self.next_loc() {
            Err(_) => None,
            Ok(loc) => loc,
        }
    }
}

#[cfg(feature = "fallible-iterator")]
impl<'ctx, R> fallible_iterator::FallibleIterator for LocationRangeIter<'ctx, R>
where
    R: gimli::Reader + 'ctx,
{
    type Item = (u64, u64, Location<'ctx>);
    type Error = Error;

    #[inline]
    fn next(&mut self) -> Result<Option<Self::Item>, Self::Error> {
        self.next_loc()
    }
}

struct LocationRangeUnitIter<'ctx> {
    lines: &'ctx Lines,
    seqs: &'ctx [LineSequence],
    seq_idx: usize,
    row_idx: usize,
    probe_high: u64,
}

impl<'ctx> LocationRangeUnitIter<'ctx> {
    fn new<R: gimli::Reader>(
        resunit: &'ctx ResUnit<R>,
        sections: &gimli::Dwarf<R>,
        probe_low: u64,
        probe_high: u64,
    ) -> Result<Option<Self>, Error> {
        let lines = resunit.parse_lines(sections)?;

        if let Some(lines) = lines {
            // Find index for probe_low.
            let seq_idx = lines.sequences.binary_search_by(|sequence| {
                if probe_low < sequence.start {
                    Ordering::Greater
                } else if probe_low >= sequence.end {
                    Ordering::Less
                } else {
                    Ordering::Equal
                }
            });
            let seq_idx = match seq_idx {
                Ok(x) => x,
                Err(0) => 0, // probe below sequence, but range could overlap
                Err(_) => lines.sequences.len(),
            };

            let row_idx = if let Some(seq) = lines.sequences.get(seq_idx) {
                let idx = seq.rows.binary_search_by(|row| row.address.cmp(&probe_low));
                match idx {
                    Ok(x) => x,
                    Err(0) => 0, // probe below sequence, but range could overlap
                    Err(x) => x - 1,
                }
            } else {
                0
            };

            Ok(Some(Self {
                lines,
                seqs: &*lines.sequences,
                seq_idx,
                row_idx,
                probe_high,
            }))
        } else {
            Ok(None)
        }
    }
}

impl<'ctx> Iterator for LocationRangeUnitIter<'ctx> {
    type Item = (u64, u64, Location<'ctx>);

    fn next(&mut self) -> Option<(u64, u64, Location<'ctx>)> {
        while let Some(seq) = self.seqs.get(self.seq_idx) {
            if seq.start >= self.probe_high {
                break;
            }

            match seq.rows.get(self.row_idx) {
                Some(row) => {
                    if row.address >= self.probe_high {
                        break;
                    }

                    let file = self
                        .lines
                        .files
                        .get(row.file_index as usize)
                        .map(String::as_str);
                    let nextaddr = seq
                        .rows
                        .get(self.row_idx + 1)
                        .map(|row| row.address)
                        .unwrap_or(seq.end);

                    let item = (
                        row.address,
                        nextaddr - row.address,
                        Location {
                            file,
                            line: if row.line != 0 { Some(row.line) } else { None },
                            column: if row.column != 0 {
                                Some(row.column)
                            } else {
                                None
                            },
                        },
                    );
                    self.row_idx += 1;

                    return Some(item);
                }
                None => {
                    self.seq_idx += 1;
                    self.row_idx = 0;
                }
            }
        }
        None
    }
}

fn path_push(path: &mut String, p: &str) {
    if has_unix_root(p) || has_windows_root(p) {
        *path = p.to_string();
    } else {
        let dir_separator = if has_windows_root(path.as_str()) {
            '\\'
        } else {
            '/'
        };

        if !path.is_empty() && !path.ends_with(dir_separator) {
            path.push(dir_separator);
        }
        *path += p;
    }
}

/// Check if the path in the given string has a unix style root
fn has_unix_root(p: &str) -> bool {
    p.starts_with('/')
}

/// Check if the path in the given string has a windows style root
fn has_windows_root(p: &str) -> bool {
    p.starts_with('\\') || p.get(1..3) == Some(":\\")
}
struct RangeAttributes<R: gimli::Reader> {
    low_pc: Option<u64>,
    high_pc: Option<u64>,
    size: Option<u64>,
    ranges_offset: Option<gimli::RangeListsOffset<<R as gimli::Reader>::Offset>>,
}

impl<R: gimli::Reader> Default for RangeAttributes<R> {
    fn default() -> Self {
        RangeAttributes {
            low_pc: None,
            high_pc: None,
            size: None,
            ranges_offset: None,
        }
    }
}

impl<R: gimli::Reader> RangeAttributes<R> {
    fn for_each_range<F: FnMut(gimli::Range)>(
        &self,
        sections: &gimli::Dwarf<R>,
        unit: &gimli::Unit<R>,
        mut f: F,
    ) -> Result<bool, Error> {
        let mut added_any = false;
        let mut add_range = |range: gimli::Range| {
            if range.begin < range.end {
                f(range);
                added_any = true
            }
        };
        if let Some(ranges_offset) = self.ranges_offset {
            let mut range_list = sections.ranges(unit, ranges_offset)?;
            while let Some(range) = range_list.next()? {
                add_range(range);
            }
        } else if let (Some(begin), Some(end)) = (self.low_pc, self.high_pc) {
            add_range(gimli::Range { begin, end });
        } else if let (Some(begin), Some(size)) = (self.low_pc, self.size) {
            add_range(gimli::Range {
                begin,
                end: begin + size,
            });
        }
        Ok(added_any)
    }
}

/// An iterator over function frames.
pub struct FrameIter<'ctx, R>(FrameIterState<'ctx, R>)
where
    R: gimli::Reader;

enum FrameIterState<'ctx, R>
where
    R: gimli::Reader,
{
    Empty,
    Location(Option<Location<'ctx>>),
    Frames(FrameIterFrames<'ctx, R>),
}

struct FrameIterFrames<'ctx, R>
where
    R: gimli::Reader,
{
    unit: &'ctx ResUnit<R>,
    sections: &'ctx gimli::Dwarf<R>,
    function: &'ctx Function<R>,
    inlined_functions: iter::Rev<maybe_small::IntoIter<&'ctx InlinedFunction<R>>>,
    next: Option<Location<'ctx>>,
}

impl<'ctx, R> FrameIter<'ctx, R>
where
    R: gimli::Reader + 'ctx,
{
    /// Advances the iterator and returns the next frame.
    pub fn next(&mut self) -> Result<Option<Frame<'ctx, R>>, Error> {
        let frames = match &mut self.0 {
            FrameIterState::Empty => return Ok(None),
            FrameIterState::Location(location) => {
                // We can't move out of a mutable reference, so use `take` instead.
                let location = location.take();
                self.0 = FrameIterState::Empty;
                return Ok(Some(Frame {
                    dw_die_offset: None,
                    function: None,
                    location,
                }));
            }
            FrameIterState::Frames(frames) => frames,
        };

        let loc = frames.next.take();
        let func = match frames.inlined_functions.next() {
            Some(func) => func,
            None => {
                let frame = Frame {
                    dw_die_offset: Some(frames.function.dw_die_offset),
                    function: frames.function.name.clone().map(|name| FunctionName {
                        name,
                        language: frames.unit.lang,
                    }),
                    location: loc,
                };
                self.0 = FrameIterState::Empty;
                return Ok(Some(frame));
            }
        };

        let mut next = Location {
            file: None,
            line: if func.call_line != 0 {
                Some(func.call_line)
            } else {
                None
            },
            column: if func.call_column != 0 {
                Some(func.call_column)
            } else {
                None
            },
        };
        if let Some(call_file) = func.call_file {
            if let Some(lines) = frames.unit.parse_lines(frames.sections)? {
                next.file = lines.files.get(call_file as usize).map(String::as_str);
            }
        }
        frames.next = Some(next);

        Ok(Some(Frame {
            dw_die_offset: Some(func.dw_die_offset),
            function: func.name.clone().map(|name| FunctionName {
                name,
                language: frames.unit.lang,
            }),
            location: loc,
        }))
    }
}

#[cfg(feature = "fallible-iterator")]
impl<'ctx, R> fallible_iterator::FallibleIterator for FrameIter<'ctx, R>
where
    R: gimli::Reader + 'ctx,
{
    type Item = Frame<'ctx, R>;
    type Error = Error;

    #[inline]
    fn next(&mut self) -> Result<Option<Frame<'ctx, R>>, Error> {
        self.next()
    }
}

/// A function frame.
pub struct Frame<'ctx, R: gimli::Reader> {
    /// The DWARF unit offset corresponding to the DIE of the function.
    pub dw_die_offset: Option<gimli::UnitOffset<R::Offset>>,
    /// The name of the function.
    pub function: Option<FunctionName<R>>,
    /// The source location corresponding to this frame.
    pub location: Option<Location<'ctx>>,
}

/// A function name.
pub struct FunctionName<R: gimli::Reader> {
    /// The name of the function.
    pub name: R,
    /// The language of the compilation unit containing this function.
    pub language: Option<gimli::DwLang>,
}

impl<R: gimli::Reader> FunctionName<R> {
    /// The raw name of this function before demangling.
    pub fn raw_name(&self) -> Result<Cow<'_, str>, Error> {
        self.name.to_string_lossy()
    }

    /// The name of this function after demangling (if applicable).
    pub fn demangle(&self) -> Result<Cow<'_, str>, Error> {
        self.raw_name().map(|x| demangle_auto(x, self.language))
    }
}

/// Demangle a symbol name using the demangling scheme for the given language.
///
/// Returns `None` if demangling failed or is not required.
#[allow(unused_variables)]
pub fn demangle(name: &str, language: gimli::DwLang) -> Option<String> {
    match language {
        #[cfg(feature = "rustc-demangle")]
        gimli::DW_LANG_Rust => rustc_demangle::try_demangle(name)
            .ok()
            .as_ref()
            .map(|x| format!("{:#}", x)),
        #[cfg(feature = "cpp_demangle")]
        gimli::DW_LANG_C_plus_plus
        | gimli::DW_LANG_C_plus_plus_03
        | gimli::DW_LANG_C_plus_plus_11
        | gimli::DW_LANG_C_plus_plus_14 => cpp_demangle::Symbol::new(name)
            .ok()
            .and_then(|x| x.demangle(&Default::default()).ok()),
        _ => None,
    }
}

/// Apply 'best effort' demangling of a symbol name.
///
/// If `language` is given, then only the demangling scheme for that language
/// is used.
///
/// If `language` is `None`, then heuristics are used to determine how to
/// demangle the name. Currently, these heuristics are very basic.
///
/// If demangling fails or is not required, then `name` is returned unchanged.
pub fn demangle_auto(name: Cow<'_, str>, language: Option<gimli::DwLang>) -> Cow<'_, str> {
    match language {
        Some(language) => demangle(name.as_ref(), language),
        None => demangle(name.as_ref(), gimli::DW_LANG_Rust)
            .or_else(|| demangle(name.as_ref(), gimli::DW_LANG_C_plus_plus)),
    }
    .map(Cow::from)
    .unwrap_or(name)
}

/// A source location.
pub struct Location<'a> {
    /// The file name.
    pub file: Option<&'a str>,
    /// The line number.
    pub line: Option<u32>,
    /// The column number.
    pub column: Option<u32>,
}

#[cfg(test)]
mod tests {
    #[test]
    fn context_is_send() {
        fn assert_is_send<T: Send>() {}
        assert_is_send::<crate::Context<gimli::read::EndianSlice<'_, gimli::LittleEndian>>>();
    }
}
