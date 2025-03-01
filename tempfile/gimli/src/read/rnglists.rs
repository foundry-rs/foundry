use crate::common::{
    DebugAddrBase, DebugAddrIndex, DebugRngListsBase, DebugRngListsIndex, DwarfFileType, Encoding,
    RangeListsOffset, SectionId,
};
use crate::constants;
use crate::endianity::Endianity;
use crate::read::{
    lists::ListsHeader, DebugAddr, EndianSlice, Error, Reader, ReaderOffset, ReaderOffsetId,
    Result, Section,
};

/// The raw contents of the `.debug_ranges` section.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebugRanges<R> {
    pub(crate) section: R,
}

impl<'input, Endian> DebugRanges<EndianSlice<'input, Endian>>
where
    Endian: Endianity,
{
    /// Construct a new `DebugRanges` instance from the data in the `.debug_ranges`
    /// section.
    ///
    /// It is the caller's responsibility to read the `.debug_ranges` section and
    /// present it as a `&[u8]` slice. That means using some ELF loader on
    /// Linux, a Mach-O loader on macOS, etc.
    ///
    /// ```
    /// use gimli::{DebugRanges, LittleEndian};
    ///
    /// # let buf = [0x00, 0x01, 0x02, 0x03];
    /// # let read_debug_ranges_section_somehow = || &buf;
    /// let debug_ranges = DebugRanges::new(read_debug_ranges_section_somehow(), LittleEndian);
    /// ```
    pub fn new(section: &'input [u8], endian: Endian) -> Self {
        Self::from(EndianSlice::new(section, endian))
    }
}

impl<R> Section<R> for DebugRanges<R> {
    fn id() -> SectionId {
        SectionId::DebugRanges
    }

    fn reader(&self) -> &R {
        &self.section
    }
}

impl<R> From<R> for DebugRanges<R> {
    fn from(section: R) -> Self {
        DebugRanges { section }
    }
}

/// The `DebugRngLists` struct represents the contents of the
/// `.debug_rnglists` section.
#[derive(Debug, Default, Clone, Copy)]
pub struct DebugRngLists<R> {
    section: R,
}

impl<'input, Endian> DebugRngLists<EndianSlice<'input, Endian>>
where
    Endian: Endianity,
{
    /// Construct a new `DebugRngLists` instance from the data in the
    /// `.debug_rnglists` section.
    ///
    /// It is the caller's responsibility to read the `.debug_rnglists`
    /// section and present it as a `&[u8]` slice. That means using some ELF
    /// loader on Linux, a Mach-O loader on macOS, etc.
    ///
    /// ```
    /// use gimli::{DebugRngLists, LittleEndian};
    ///
    /// # let buf = [0x00, 0x01, 0x02, 0x03];
    /// # let read_debug_rnglists_section_somehow = || &buf;
    /// let debug_rnglists =
    ///     DebugRngLists::new(read_debug_rnglists_section_somehow(), LittleEndian);
    /// ```
    pub fn new(section: &'input [u8], endian: Endian) -> Self {
        Self::from(EndianSlice::new(section, endian))
    }
}

impl<R> Section<R> for DebugRngLists<R> {
    fn id() -> SectionId {
        SectionId::DebugRngLists
    }

    fn reader(&self) -> &R {
        &self.section
    }
}

impl<R> From<R> for DebugRngLists<R> {
    fn from(section: R) -> Self {
        DebugRngLists { section }
    }
}

#[allow(unused)]
pub(crate) type RngListsHeader = ListsHeader;

impl<Offset> DebugRngListsBase<Offset>
where
    Offset: ReaderOffset,
{
    /// Returns a `DebugRngListsBase` with the default value of DW_AT_rnglists_base
    /// for the given `Encoding` and `DwarfFileType`.
    pub fn default_for_encoding_and_file(
        encoding: Encoding,
        file_type: DwarfFileType,
    ) -> DebugRngListsBase<Offset> {
        if encoding.version >= 5 && file_type == DwarfFileType::Dwo {
            // In .dwo files, the compiler omits the DW_AT_rnglists_base attribute (because there is
            // only a single unit in the file) but we must skip past the header, which the attribute
            // would normally do for us.
            DebugRngListsBase(Offset::from_u8(RngListsHeader::size_for_encoding(encoding)))
        } else {
            DebugRngListsBase(Offset::from_u8(0))
        }
    }
}

/// The DWARF data found in `.debug_ranges` and `.debug_rnglists` sections.
#[derive(Debug, Default, Clone, Copy)]
pub struct RangeLists<R> {
    debug_ranges: DebugRanges<R>,
    debug_rnglists: DebugRngLists<R>,
}

impl<R> RangeLists<R> {
    /// Construct a new `RangeLists` instance from the data in the `.debug_ranges` and
    /// `.debug_rnglists` sections.
    pub fn new(debug_ranges: DebugRanges<R>, debug_rnglists: DebugRngLists<R>) -> RangeLists<R> {
        RangeLists {
            debug_ranges,
            debug_rnglists,
        }
    }

    /// Return the `.debug_ranges` section.
    pub fn debug_ranges(&self) -> &DebugRanges<R> {
        &self.debug_ranges
    }

    /// Replace the `.debug_ranges` section.
    ///
    /// This is useful for `.dwo` files when using the GNU split-dwarf extension to DWARF 4.
    pub fn set_debug_ranges(&mut self, debug_ranges: DebugRanges<R>) {
        self.debug_ranges = debug_ranges;
    }

    /// Return the `.debug_rnglists` section.
    pub fn debug_rnglists(&self) -> &DebugRngLists<R> {
        &self.debug_rnglists
    }
}

impl<T> RangeLists<T> {
    /// Create a `RangeLists` that references the data in `self`.
    ///
    /// This is useful when `R` implements `Reader` but `T` does not.
    ///
    /// ## Example Usage
    ///
    /// ```rust,no_run
    /// # let load_section = || unimplemented!();
    /// // Read the DWARF section into a `Vec` with whatever object loader you're using.
    /// let owned_section: gimli::RangeLists<Vec<u8>> = load_section();
    /// // Create a reference to the DWARF section.
    /// let section = owned_section.borrow(|section| {
    ///     gimli::EndianSlice::new(&section, gimli::LittleEndian)
    /// });
    /// ```
    pub fn borrow<'a, F, R>(&'a self, mut borrow: F) -> RangeLists<R>
    where
        F: FnMut(&'a T) -> R,
    {
        RangeLists {
            debug_ranges: borrow(&self.debug_ranges.section).into(),
            debug_rnglists: borrow(&self.debug_rnglists.section).into(),
        }
    }
}

impl<R: Reader> RangeLists<R> {
    /// Iterate over the `Range` list entries starting at the given offset.
    ///
    /// The `unit_version` and `address_size` must match the compilation unit that the
    /// offset was contained in.
    ///
    /// The `base_address` should be obtained from the `DW_AT_low_pc` attribute in the
    /// `DW_TAG_compile_unit` entry for the compilation unit that contains this range list.
    ///
    /// Can be [used with
    /// `FallibleIterator`](./index.html#using-with-fallibleiterator).
    pub fn ranges(
        &self,
        offset: RangeListsOffset<R::Offset>,
        unit_encoding: Encoding,
        base_address: u64,
        debug_addr: &DebugAddr<R>,
        debug_addr_base: DebugAddrBase<R::Offset>,
    ) -> Result<RngListIter<R>> {
        Ok(RngListIter::new(
            self.raw_ranges(offset, unit_encoding)?,
            base_address,
            debug_addr.clone(),
            debug_addr_base,
        ))
    }

    /// Iterate over the `RawRngListEntry`ies starting at the given offset.
    ///
    /// The `unit_encoding` must match the compilation unit that the
    /// offset was contained in.
    ///
    /// This iterator does not perform any processing of the range entries,
    /// such as handling base addresses.
    ///
    /// Can be [used with
    /// `FallibleIterator`](./index.html#using-with-fallibleiterator).
    pub fn raw_ranges(
        &self,
        offset: RangeListsOffset<R::Offset>,
        unit_encoding: Encoding,
    ) -> Result<RawRngListIter<R>> {
        let (mut input, format) = if unit_encoding.version <= 4 {
            (self.debug_ranges.section.clone(), RangeListsFormat::Bare)
        } else {
            (self.debug_rnglists.section.clone(), RangeListsFormat::Rle)
        };
        input.skip(offset.0)?;
        Ok(RawRngListIter::new(input, unit_encoding, format))
    }

    /// Returns the `.debug_rnglists` offset at the given `base` and `index`.
    ///
    /// The `base` must be the `DW_AT_rnglists_base` value from the compilation unit DIE.
    /// This is an offset that points to the first entry following the header.
    ///
    /// The `index` is the value of a `DW_FORM_rnglistx` attribute.
    ///
    /// The `unit_encoding` must match the compilation unit that the
    /// index was contained in.
    pub fn get_offset(
        &self,
        unit_encoding: Encoding,
        base: DebugRngListsBase<R::Offset>,
        index: DebugRngListsIndex<R::Offset>,
    ) -> Result<RangeListsOffset<R::Offset>> {
        let format = unit_encoding.format;
        let input = &mut self.debug_rnglists.section.clone();
        input.skip(base.0)?;
        input.skip(R::Offset::from_u64(
            index.0.into_u64() * u64::from(format.word_size()),
        )?)?;
        input
            .read_offset(format)
            .map(|x| RangeListsOffset(base.0 + x))
    }

    /// Call `Reader::lookup_offset_id` for each section, and return the first match.
    pub fn lookup_offset_id(&self, id: ReaderOffsetId) -> Option<(SectionId, R::Offset)> {
        self.debug_ranges
            .lookup_offset_id(id)
            .or_else(|| self.debug_rnglists.lookup_offset_id(id))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RangeListsFormat {
    /// The bare range list format used before DWARF 5.
    Bare,
    /// The DW_RLE encoded range list format used in DWARF 5.
    Rle,
}

/// A raw iterator over an address range list.
///
/// This iterator does not perform any processing of the range entries,
/// such as handling base addresses.
#[derive(Debug)]
pub struct RawRngListIter<R: Reader> {
    input: R,
    encoding: Encoding,
    format: RangeListsFormat,
}

/// A raw entry in .debug_rnglists
#[derive(Clone, Debug)]
pub enum RawRngListEntry<T> {
    /// A range from DWARF version <= 4.
    AddressOrOffsetPair {
        /// Start of range. May be an address or an offset.
        begin: u64,
        /// End of range. May be an address or an offset.
        end: u64,
    },
    /// DW_RLE_base_address
    BaseAddress {
        /// base address
        addr: u64,
    },
    /// DW_RLE_base_addressx
    BaseAddressx {
        /// base address
        addr: DebugAddrIndex<T>,
    },
    /// DW_RLE_startx_endx
    StartxEndx {
        /// start of range
        begin: DebugAddrIndex<T>,
        /// end of range
        end: DebugAddrIndex<T>,
    },
    /// DW_RLE_startx_length
    StartxLength {
        /// start of range
        begin: DebugAddrIndex<T>,
        /// length of range
        length: u64,
    },
    /// DW_RLE_offset_pair
    OffsetPair {
        /// start of range
        begin: u64,
        /// end of range
        end: u64,
    },
    /// DW_RLE_start_end
    StartEnd {
        /// start of range
        begin: u64,
        /// end of range
        end: u64,
    },
    /// DW_RLE_start_length
    StartLength {
        /// start of range
        begin: u64,
        /// length of range
        length: u64,
    },
}

impl<T: ReaderOffset> RawRngListEntry<T> {
    /// Parse a range entry from `.debug_rnglists`
    fn parse<R: Reader<Offset = T>>(
        input: &mut R,
        encoding: Encoding,
        format: RangeListsFormat,
    ) -> Result<Option<Self>> {
        Ok(match format {
            RangeListsFormat::Bare => {
                let range = RawRange::parse(input, encoding.address_size)?;
                if range.is_end() {
                    None
                } else if range.is_base_address(encoding.address_size) {
                    Some(RawRngListEntry::BaseAddress { addr: range.end })
                } else {
                    Some(RawRngListEntry::AddressOrOffsetPair {
                        begin: range.begin,
                        end: range.end,
                    })
                }
            }
            RangeListsFormat::Rle => match constants::DwRle(input.read_u8()?) {
                constants::DW_RLE_end_of_list => None,
                constants::DW_RLE_base_addressx => Some(RawRngListEntry::BaseAddressx {
                    addr: DebugAddrIndex(input.read_uleb128().and_then(R::Offset::from_u64)?),
                }),
                constants::DW_RLE_startx_endx => Some(RawRngListEntry::StartxEndx {
                    begin: DebugAddrIndex(input.read_uleb128().and_then(R::Offset::from_u64)?),
                    end: DebugAddrIndex(input.read_uleb128().and_then(R::Offset::from_u64)?),
                }),
                constants::DW_RLE_startx_length => Some(RawRngListEntry::StartxLength {
                    begin: DebugAddrIndex(input.read_uleb128().and_then(R::Offset::from_u64)?),
                    length: input.read_uleb128()?,
                }),
                constants::DW_RLE_offset_pair => Some(RawRngListEntry::OffsetPair {
                    begin: input.read_uleb128()?,
                    end: input.read_uleb128()?,
                }),
                constants::DW_RLE_base_address => Some(RawRngListEntry::BaseAddress {
                    addr: input.read_address(encoding.address_size)?,
                }),
                constants::DW_RLE_start_end => Some(RawRngListEntry::StartEnd {
                    begin: input.read_address(encoding.address_size)?,
                    end: input.read_address(encoding.address_size)?,
                }),
                constants::DW_RLE_start_length => Some(RawRngListEntry::StartLength {
                    begin: input.read_address(encoding.address_size)?,
                    length: input.read_uleb128()?,
                }),
                _ => {
                    return Err(Error::InvalidAddressRange);
                }
            },
        })
    }
}

impl<R: Reader> RawRngListIter<R> {
    /// Construct a `RawRngListIter`.
    fn new(input: R, encoding: Encoding, format: RangeListsFormat) -> RawRngListIter<R> {
        RawRngListIter {
            input,
            encoding,
            format,
        }
    }

    /// Advance the iterator to the next range.
    pub fn next(&mut self) -> Result<Option<RawRngListEntry<R::Offset>>> {
        if self.input.is_empty() {
            return Ok(None);
        }

        match RawRngListEntry::parse(&mut self.input, self.encoding, self.format) {
            Ok(range) => {
                if range.is_none() {
                    self.input.empty();
                }
                Ok(range)
            }
            Err(e) => {
                self.input.empty();
                Err(e)
            }
        }
    }
}

#[cfg(feature = "fallible-iterator")]
impl<R: Reader> fallible_iterator::FallibleIterator for RawRngListIter<R> {
    type Item = RawRngListEntry<R::Offset>;
    type Error = Error;

    fn next(&mut self) -> ::core::result::Result<Option<Self::Item>, Self::Error> {
        RawRngListIter::next(self)
    }
}

/// An iterator over an address range list.
///
/// This iterator internally handles processing of base addresses and different
/// entry types.  Thus, it only returns range entries that are valid
/// and already adjusted for the base address.
#[derive(Debug)]
pub struct RngListIter<R: Reader> {
    raw: RawRngListIter<R>,
    base_address: u64,
    debug_addr: DebugAddr<R>,
    debug_addr_base: DebugAddrBase<R::Offset>,
}

impl<R: Reader> RngListIter<R> {
    /// Construct a `RngListIter`.
    fn new(
        raw: RawRngListIter<R>,
        base_address: u64,
        debug_addr: DebugAddr<R>,
        debug_addr_base: DebugAddrBase<R::Offset>,
    ) -> RngListIter<R> {
        RngListIter {
            raw,
            base_address,
            debug_addr,
            debug_addr_base,
        }
    }

    #[inline]
    fn get_address(&self, index: DebugAddrIndex<R::Offset>) -> Result<u64> {
        self.debug_addr
            .get_address(self.raw.encoding.address_size, self.debug_addr_base, index)
    }

    /// Advance the iterator to the next range.
    pub fn next(&mut self) -> Result<Option<Range>> {
        loop {
            let raw_range = match self.raw.next()? {
                Some(range) => range,
                None => return Ok(None),
            };

            let range = self.convert_raw(raw_range)?;
            if range.is_some() {
                return Ok(range);
            }
        }
    }

    /// Return the next raw range.
    ///
    /// The raw range should be passed to `convert_range`.
    #[doc(hidden)]
    pub fn next_raw(&mut self) -> Result<Option<RawRngListEntry<R::Offset>>> {
        self.raw.next()
    }

    /// Convert a raw range into a range, and update the state of the iterator.
    ///
    /// The raw range should have been obtained from `next_raw`.
    #[doc(hidden)]
    pub fn convert_raw(&mut self, raw_range: RawRngListEntry<R::Offset>) -> Result<Option<Range>> {
        let mask = !0 >> (64 - self.raw.encoding.address_size * 8);
        let tombstone = if self.raw.encoding.version <= 4 {
            mask - 1
        } else {
            mask
        };

        let range = match raw_range {
            RawRngListEntry::BaseAddress { addr } => {
                self.base_address = addr;
                return Ok(None);
            }
            RawRngListEntry::BaseAddressx { addr } => {
                self.base_address = self.get_address(addr)?;
                return Ok(None);
            }
            RawRngListEntry::StartxEndx { begin, end } => {
                let begin = self.get_address(begin)?;
                let end = self.get_address(end)?;
                Range { begin, end }
            }
            RawRngListEntry::StartxLength { begin, length } => {
                let begin = self.get_address(begin)?;
                let end = begin.wrapping_add(length) & mask;
                Range { begin, end }
            }
            RawRngListEntry::AddressOrOffsetPair { begin, end }
            | RawRngListEntry::OffsetPair { begin, end } => {
                if self.base_address == tombstone {
                    return Ok(None);
                }
                let mut range = Range { begin, end };
                range.add_base_address(self.base_address, self.raw.encoding.address_size);
                range
            }
            RawRngListEntry::StartEnd { begin, end } => Range { begin, end },
            RawRngListEntry::StartLength { begin, length } => {
                let end = begin.wrapping_add(length) & mask;
                Range { begin, end }
            }
        };

        if range.begin == tombstone {
            return Ok(None);
        }

        if range.begin > range.end {
            self.raw.input.empty();
            return Err(Error::InvalidAddressRange);
        }

        Ok(Some(range))
    }
}

#[cfg(feature = "fallible-iterator")]
impl<R: Reader> fallible_iterator::FallibleIterator for RngListIter<R> {
    type Item = Range;
    type Error = Error;

    fn next(&mut self) -> ::core::result::Result<Option<Self::Item>, Self::Error> {
        RngListIter::next(self)
    }
}

/// A raw address range from the `.debug_ranges` section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct RawRange {
    /// The beginning address of the range.
    pub begin: u64,

    /// The first address past the end of the range.
    pub end: u64,
}

impl RawRange {
    /// Check if this is a range end entry.
    #[inline]
    pub fn is_end(&self) -> bool {
        self.begin == 0 && self.end == 0
    }

    /// Check if this is a base address selection entry.
    ///
    /// A base address selection entry changes the base address that subsequent
    /// range entries are relative to.
    #[inline]
    pub fn is_base_address(&self, address_size: u8) -> bool {
        self.begin == !0 >> (64 - address_size * 8)
    }

    /// Parse an address range entry from `.debug_ranges` or `.debug_loc`.
    #[inline]
    pub fn parse<R: Reader>(input: &mut R, address_size: u8) -> Result<RawRange> {
        let begin = input.read_address(address_size)?;
        let end = input.read_address(address_size)?;
        let range = RawRange { begin, end };
        Ok(range)
    }
}

/// An address range from the `.debug_ranges`, `.debug_rnglists`, or `.debug_aranges` sections.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Range {
    /// The beginning address of the range.
    pub begin: u64,

    /// The first address past the end of the range.
    pub end: u64,
}

impl Range {
    /// Add a base address to this range.
    #[inline]
    pub(crate) fn add_base_address(&mut self, base_address: u64, address_size: u8) {
        let mask = !0 >> (64 - address_size * 8);
        self.begin = base_address.wrapping_add(self.begin) & mask;
        self.end = base_address.wrapping_add(self.end) & mask;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Format;
    use crate::endianity::LittleEndian;
    use crate::test_util::GimliSectionMethods;
    use test_assembler::{Endian, Label, LabelMaker, Section};

    #[test]
    fn test_rnglists_32() {
        let tombstone = !0u32;
        let encoding = Encoding {
            format: Format::Dwarf32,
            version: 5,
            address_size: 4,
        };
        let section = Section::with_endian(Endian::Little)
            .L32(0x0300_0000)
            .L32(0x0301_0300)
            .L32(0x0301_0400)
            .L32(0x0301_0500)
            .L32(tombstone)
            .L32(0x0301_0600);
        let buf = section.get_contents().unwrap();
        let debug_addr = &DebugAddr::from(EndianSlice::new(&buf, LittleEndian));
        let debug_addr_base = DebugAddrBase(0);

        let start = Label::new();
        let first = Label::new();
        let size = Label::new();
        #[rustfmt::skip]
        let section = Section::with_endian(Endian::Little)
            // Header
            .mark(&start)
            .L32(&size)
            .L16(encoding.version)
            .L8(encoding.address_size)
            .L8(0)
            .L32(0)
            .mark(&first)
            // An OffsetPair using the unit base address.
            .L8(4).uleb(0x10200).uleb(0x10300)
            // A base address selection followed by an OffsetPair.
            .L8(5).L32(0x0200_0000)
            .L8(4).uleb(0x10400).uleb(0x10500)
            // An empty OffsetPair followed by a normal OffsetPair.
            .L8(4).uleb(0x10600).uleb(0x10600)
            .L8(4).uleb(0x10800).uleb(0x10900)
            // A StartEnd
            .L8(6).L32(0x201_0a00).L32(0x201_0b00)
            // A StartLength
            .L8(7).L32(0x201_0c00).uleb(0x100)
            // An OffsetPair that starts at 0.
            .L8(4).uleb(0).uleb(1)
            // An OffsetPair that starts and ends at 0.
            .L8(4).uleb(0).uleb(0)
            // An OffsetPair that ends at -1.
            .L8(5).L32(0)
            .L8(4).uleb(0).uleb(0xffff_ffff)
            // A BaseAddressx + OffsetPair
            .L8(1).uleb(0)
            .L8(4).uleb(0x10100).uleb(0x10200)
            // A StartxEndx
            .L8(2).uleb(1).uleb(2)
            // A StartxLength
            .L8(3).uleb(3).uleb(0x100)

            // Tombstone entries, all of which should be ignored.
            // A BaseAddressx that is a tombstone.
            .L8(1).uleb(4)
            .L8(4).uleb(0x11100).uleb(0x11200)
            // A BaseAddress that is a tombstone.
            .L8(5).L32(tombstone)
            .L8(4).uleb(0x11300).uleb(0x11400)
            // A StartxEndx that is a tombstone.
            .L8(2).uleb(4).uleb(5)
            // A StartxLength that is a tombstone.
            .L8(3).uleb(4).uleb(0x100)
            // A StartEnd that is a tombstone.
            .L8(6).L32(tombstone).L32(0x201_1500)
            // A StartLength that is a tombstone.
            .L8(7).L32(tombstone).uleb(0x100)
            // A StartEnd (not ignored)
            .L8(6).L32(0x201_1600).L32(0x201_1700)

            // A range end.
            .L8(0)
            // Some extra data.
            .L32(0xffff_ffff);
        size.set_const((&section.here() - &start - 4) as u64);

        let buf = section.get_contents().unwrap();
        let debug_ranges = DebugRanges::new(&[], LittleEndian);
        let debug_rnglists = DebugRngLists::new(&buf, LittleEndian);
        let rnglists = RangeLists::new(debug_ranges, debug_rnglists);
        let offset = RangeListsOffset((&first - &start) as usize);
        let mut ranges = rnglists
            .ranges(offset, encoding, 0x0100_0000, debug_addr, debug_addr_base)
            .unwrap();

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0101_0200,
                end: 0x0101_0300,
            }))
        );

        // A base address selection followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0400,
                end: 0x0201_0500,
            }))
        );

        // An empty range followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0600,
                end: 0x0201_0600,
            }))
        );
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0800,
                end: 0x0201_0900,
            }))
        );

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0a00,
                end: 0x0201_0b00,
            }))
        );

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0c00,
                end: 0x0201_0d00,
            }))
        );

        // A range that starts at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0001,
            }))
        );

        // A range that starts and ends at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0000,
            }))
        );

        // A range that ends at -1.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0000_0000,
                end: 0xffff_ffff,
            }))
        );

        // A BaseAddressx + OffsetPair
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0100,
                end: 0x0301_0200,
            }))
        );

        // A StartxEndx
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0300,
                end: 0x0301_0400,
            }))
        );

        // A StartxLength
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0500,
                end: 0x0301_0600,
            }))
        );

        // A StartEnd range following the tombstones
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_1600,
                end: 0x0201_1700,
            }))
        );

        // A range end.
        assert_eq!(ranges.next(), Ok(None));

        // An offset at the end of buf.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(buf.len()),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Ok(None));
    }

    #[test]
    fn test_rnglists_64() {
        let tombstone = !0u64;
        let encoding = Encoding {
            format: Format::Dwarf64,
            version: 5,
            address_size: 8,
        };
        let section = Section::with_endian(Endian::Little)
            .L64(0x0300_0000)
            .L64(0x0301_0300)
            .L64(0x0301_0400)
            .L64(0x0301_0500)
            .L64(tombstone)
            .L64(0x0301_0600);
        let buf = section.get_contents().unwrap();
        let debug_addr = &DebugAddr::from(EndianSlice::new(&buf, LittleEndian));
        let debug_addr_base = DebugAddrBase(0);

        let start = Label::new();
        let first = Label::new();
        let size = Label::new();
        #[rustfmt::skip]
        let section = Section::with_endian(Endian::Little)
            // Header
            .mark(&start)
            .L32(0xffff_ffff)
            .L64(&size)
            .L16(encoding.version)
            .L8(encoding.address_size)
            .L8(0)
            .L32(0)
            .mark(&first)
            // An OffsetPair using the unit base address.
            .L8(4).uleb(0x10200).uleb(0x10300)
            // A base address selection followed by an OffsetPair.
            .L8(5).L64(0x0200_0000)
            .L8(4).uleb(0x10400).uleb(0x10500)
            // An empty OffsetPair followed by a normal OffsetPair.
            .L8(4).uleb(0x10600).uleb(0x10600)
            .L8(4).uleb(0x10800).uleb(0x10900)
            // A StartEnd
            .L8(6).L64(0x201_0a00).L64(0x201_0b00)
            // A StartLength
            .L8(7).L64(0x201_0c00).uleb(0x100)
            // An OffsetPair that starts at 0.
            .L8(4).uleb(0).uleb(1)
            // An OffsetPair that starts and ends at 0.
            .L8(4).uleb(0).uleb(0)
            // An OffsetPair that ends at -1.
            .L8(5).L64(0)
            .L8(4).uleb(0).uleb(0xffff_ffff)
            // A BaseAddressx + OffsetPair
            .L8(1).uleb(0)
            .L8(4).uleb(0x10100).uleb(0x10200)
            // A StartxEndx
            .L8(2).uleb(1).uleb(2)
            // A StartxLength
            .L8(3).uleb(3).uleb(0x100)

            // Tombstone entries, all of which should be ignored.
            // A BaseAddressx that is a tombstone.
            .L8(1).uleb(4)
            .L8(4).uleb(0x11100).uleb(0x11200)
            // A BaseAddress that is a tombstone.
            .L8(5).L64(tombstone)
            .L8(4).uleb(0x11300).uleb(0x11400)
            // A StartxEndx that is a tombstone.
            .L8(2).uleb(4).uleb(5)
            // A StartxLength that is a tombstone.
            .L8(3).uleb(4).uleb(0x100)
            // A StartEnd that is a tombstone.
            .L8(6).L64(tombstone).L64(0x201_1500)
            // A StartLength that is a tombstone.
            .L8(7).L64(tombstone).uleb(0x100)
            // A StartEnd (not ignored)
            .L8(6).L64(0x201_1600).L64(0x201_1700)

            // A range end.
            .L8(0)
            // Some extra data.
            .L32(0xffff_ffff);
        size.set_const((&section.here() - &start - 12) as u64);

        let buf = section.get_contents().unwrap();
        let debug_ranges = DebugRanges::new(&[], LittleEndian);
        let debug_rnglists = DebugRngLists::new(&buf, LittleEndian);
        let rnglists = RangeLists::new(debug_ranges, debug_rnglists);
        let offset = RangeListsOffset((&first - &start) as usize);
        let mut ranges = rnglists
            .ranges(offset, encoding, 0x0100_0000, debug_addr, debug_addr_base)
            .unwrap();

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0101_0200,
                end: 0x0101_0300,
            }))
        );

        // A base address selection followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0400,
                end: 0x0201_0500,
            }))
        );

        // An empty range followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0600,
                end: 0x0201_0600,
            }))
        );
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0800,
                end: 0x0201_0900,
            }))
        );

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0a00,
                end: 0x0201_0b00,
            }))
        );

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0c00,
                end: 0x0201_0d00,
            }))
        );

        // A range that starts at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0001,
            }))
        );

        // A range that starts and ends at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0000,
            }))
        );

        // A range that ends at -1.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0000_0000,
                end: 0xffff_ffff,
            }))
        );

        // A BaseAddressx + OffsetPair
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0100,
                end: 0x0301_0200,
            }))
        );

        // A StartxEndx
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0300,
                end: 0x0301_0400,
            }))
        );

        // A StartxLength
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0301_0500,
                end: 0x0301_0600,
            }))
        );

        // A StartEnd range following the tombstones
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_1600,
                end: 0x0201_1700,
            }))
        );

        // A range end.
        assert_eq!(ranges.next(), Ok(None));

        // An offset at the end of buf.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(buf.len()),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Ok(None));
    }

    #[test]
    fn test_raw_range() {
        let range = RawRange {
            begin: 0,
            end: 0xffff_ffff,
        };
        assert!(!range.is_end());
        assert!(!range.is_base_address(4));
        assert!(!range.is_base_address(8));

        let range = RawRange { begin: 0, end: 0 };
        assert!(range.is_end());
        assert!(!range.is_base_address(4));
        assert!(!range.is_base_address(8));

        let range = RawRange {
            begin: 0xffff_ffff,
            end: 0,
        };
        assert!(!range.is_end());
        assert!(range.is_base_address(4));
        assert!(!range.is_base_address(8));

        let range = RawRange {
            begin: 0xffff_ffff_ffff_ffff,
            end: 0,
        };
        assert!(!range.is_end());
        assert!(!range.is_base_address(4));
        assert!(range.is_base_address(8));
    }

    #[test]
    fn test_ranges_32() {
        let tombstone = !0u32 - 1;
        let start = Label::new();
        let first = Label::new();
        #[rustfmt::skip]
        let section = Section::with_endian(Endian::Little)
            // A range before the offset.
            .mark(&start)
            .L32(0x10000).L32(0x10100)
            .mark(&first)
            // A normal range.
            .L32(0x10200).L32(0x10300)
            // A base address selection followed by a normal range.
            .L32(0xffff_ffff).L32(0x0200_0000)
            .L32(0x10400).L32(0x10500)
            // An empty range followed by a normal range.
            .L32(0x10600).L32(0x10600)
            .L32(0x10800).L32(0x10900)
            // A range that starts at 0.
            .L32(0).L32(1)
            // A range that ends at -1.
            .L32(0xffff_ffff).L32(0x0000_0000)
            .L32(0).L32(0xffff_ffff)
            // A normal range with tombstone.
            .L32(tombstone).L32(tombstone)
            // A base address selection with tombstone followed by a normal range.
            .L32(0xffff_ffff).L32(tombstone)
            .L32(0x10a00).L32(0x10b00)
            // A range end.
            .L32(0).L32(0)
            // Some extra data.
            .L32(0);

        let buf = section.get_contents().unwrap();
        let debug_ranges = DebugRanges::new(&buf, LittleEndian);
        let debug_rnglists = DebugRngLists::new(&[], LittleEndian);
        let rnglists = RangeLists::new(debug_ranges, debug_rnglists);
        let offset = RangeListsOffset((&first - &start) as usize);
        let debug_addr = &DebugAddr::from(EndianSlice::new(&[], LittleEndian));
        let debug_addr_base = DebugAddrBase(0);
        let encoding = Encoding {
            format: Format::Dwarf32,
            version: 4,
            address_size: 4,
        };
        let mut ranges = rnglists
            .ranges(offset, encoding, 0x0100_0000, debug_addr, debug_addr_base)
            .unwrap();

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0101_0200,
                end: 0x0101_0300,
            }))
        );

        // A base address selection followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0400,
                end: 0x0201_0500,
            }))
        );

        // An empty range followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0600,
                end: 0x0201_0600,
            }))
        );
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0800,
                end: 0x0201_0900,
            }))
        );

        // A range that starts at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0001,
            }))
        );

        // A range that ends at -1.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0000_0000,
                end: 0xffff_ffff,
            }))
        );

        // A range end.
        assert_eq!(ranges.next(), Ok(None));

        // An offset at the end of buf.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(buf.len()),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Ok(None));
    }

    #[test]
    fn test_ranges_64() {
        let tombstone = !0u64 - 1;
        let start = Label::new();
        let first = Label::new();
        #[rustfmt::skip]
        let section = Section::with_endian(Endian::Little)
            // A range before the offset.
            .mark(&start)
            .L64(0x10000).L64(0x10100)
            .mark(&first)
            // A normal range.
            .L64(0x10200).L64(0x10300)
            // A base address selection followed by a normal range.
            .L64(0xffff_ffff_ffff_ffff).L64(0x0200_0000)
            .L64(0x10400).L64(0x10500)
            // An empty range followed by a normal range.
            .L64(0x10600).L64(0x10600)
            .L64(0x10800).L64(0x10900)
            // A range that starts at 0.
            .L64(0).L64(1)
            // A range that ends at -1.
            .L64(0xffff_ffff_ffff_ffff).L64(0x0000_0000)
            .L64(0).L64(0xffff_ffff_ffff_ffff)
            // A normal range with tombstone.
            .L64(tombstone).L64(tombstone)
            // A base address selection with tombstone followed by a normal range.
            .L64(0xffff_ffff_ffff_ffff).L64(tombstone)
            .L64(0x10a00).L64(0x10b00)
            // A range end.
            .L64(0).L64(0)
            // Some extra data.
            .L64(0);

        let buf = section.get_contents().unwrap();
        let debug_ranges = DebugRanges::new(&buf, LittleEndian);
        let debug_rnglists = DebugRngLists::new(&[], LittleEndian);
        let rnglists = RangeLists::new(debug_ranges, debug_rnglists);
        let offset = RangeListsOffset((&first - &start) as usize);
        let debug_addr = &DebugAddr::from(EndianSlice::new(&[], LittleEndian));
        let debug_addr_base = DebugAddrBase(0);
        let encoding = Encoding {
            format: Format::Dwarf64,
            version: 4,
            address_size: 8,
        };
        let mut ranges = rnglists
            .ranges(offset, encoding, 0x0100_0000, debug_addr, debug_addr_base)
            .unwrap();

        // A normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0101_0200,
                end: 0x0101_0300,
            }))
        );

        // A base address selection followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0400,
                end: 0x0201_0500,
            }))
        );

        // An empty range followed by a normal range.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0600,
                end: 0x0201_0600,
            }))
        );
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0201_0800,
                end: 0x0201_0900,
            }))
        );

        // A range that starts at 0.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0200_0000,
                end: 0x0200_0001,
            }))
        );

        // A range that ends at -1.
        assert_eq!(
            ranges.next(),
            Ok(Some(Range {
                begin: 0x0,
                end: 0xffff_ffff_ffff_ffff,
            }))
        );

        // A range end.
        assert_eq!(ranges.next(), Ok(None));

        // An offset at the end of buf.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(buf.len()),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Ok(None));
    }

    #[test]
    fn test_ranges_invalid() {
        #[rustfmt::skip]
        let section = Section::with_endian(Endian::Little)
            // An invalid range.
            .L32(0x20000).L32(0x10000)
            // An invalid range after wrapping.
            .L32(0x20000).L32(0xff01_0000);

        let buf = section.get_contents().unwrap();
        let debug_ranges = DebugRanges::new(&buf, LittleEndian);
        let debug_rnglists = DebugRngLists::new(&[], LittleEndian);
        let rnglists = RangeLists::new(debug_ranges, debug_rnglists);
        let debug_addr = &DebugAddr::from(EndianSlice::new(&[], LittleEndian));
        let debug_addr_base = DebugAddrBase(0);
        let encoding = Encoding {
            format: Format::Dwarf32,
            version: 4,
            address_size: 4,
        };

        // An invalid range.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(0x0),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Err(Error::InvalidAddressRange));

        // An invalid range after wrapping.
        let mut ranges = rnglists
            .ranges(
                RangeListsOffset(0x8),
                encoding,
                0x0100_0000,
                debug_addr,
                debug_addr_base,
            )
            .unwrap();
        assert_eq!(ranges.next(), Err(Error::InvalidAddressRange));

        // An invalid offset.
        match rnglists.ranges(
            RangeListsOffset(buf.len() + 1),
            encoding,
            0x0100_0000,
            debug_addr,
            debug_addr_base,
        ) {
            Err(Error::UnexpectedEof(_)) => {}
            otherwise => panic!("Unexpected result: {:?}", otherwise),
        }
    }

    #[test]
    fn test_get_offset() {
        for format in vec![Format::Dwarf32, Format::Dwarf64] {
            let encoding = Encoding {
                format,
                version: 5,
                address_size: 4,
            };

            let zero = Label::new();
            let length = Label::new();
            let start = Label::new();
            let first = Label::new();
            let end = Label::new();
            let mut section = Section::with_endian(Endian::Little)
                .mark(&zero)
                .initial_length(format, &length, &start)
                .D16(encoding.version)
                .D8(encoding.address_size)
                .D8(0)
                .D32(20)
                .mark(&first);
            for i in 0..20 {
                section = section.word(format.word_size(), 1000 + i);
            }
            section = section.mark(&end);
            length.set_const((&end - &start) as u64);
            let section = section.get_contents().unwrap();

            let debug_ranges = DebugRanges::from(EndianSlice::new(&[], LittleEndian));
            let debug_rnglists = DebugRngLists::from(EndianSlice::new(&section, LittleEndian));
            let ranges = RangeLists::new(debug_ranges, debug_rnglists);

            let base = DebugRngListsBase((&first - &zero) as usize);
            assert_eq!(
                ranges.get_offset(encoding, base, DebugRngListsIndex(0)),
                Ok(RangeListsOffset(base.0 + 1000))
            );
            assert_eq!(
                ranges.get_offset(encoding, base, DebugRngListsIndex(19)),
                Ok(RangeListsOffset(base.0 + 1019))
            );
        }
    }
}
