//! Writers for the files LLVM's tools read: the coverage mapping object (`__llvm_covmap`,
//! `__llvm_covfun`, and `__llvm_prf_names` in an ELF relocatable) and the text profile.
//!
//! The coverage mapping follows `CoverageMappingWriter.cpp` and the coverage mapping version 7,
//! the newest version that LLVM 22 and 23 both read.

use super::mapping::{Counter, Expression, FunctionInput, McdcParams, RegionKind};
use std::{collections::BTreeMap, fmt::Write as _};

/// The coverage mapping version, `CovMapVersion::Version7`.
const COVMAP_VERSION: u32 = 6;

/// The separator between names in the `__llvm_prf_names` section.
const NAME_SEPARATOR: char = '\u{1}';

/// The files of a coverage mapping object.
#[derive(Debug)]
pub struct CoverageObject {
    /// The ELF relocatable object.
    pub object: Vec<u8>,
    /// The text instrumentation profile.
    pub proftext: String,
}

impl CoverageObject {
    /// Writes the coverage mapping object and the text profile of the functions.
    pub fn new(functions: &[FunctionInput]) -> eyre::Result<Self> {
        Ok(Self { object: write_object(functions)?, proftext: write_proftext(functions)? })
    }
}

/// Encodes an unsigned LEB128 value.
fn uleb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// The low 64 bits of the MD5 of `data`, as LLVM's `MD5Hash` returns them.
pub fn md5_low(data: &[u8]) -> u64 {
    let digest = md5(data);
    u64::from_le_bytes(digest[..8].try_into().unwrap())
}

/// Computes the MD5 digest of `data`.
fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    let k =
        std::array::from_fn::<u32, 64, _>(|i| ((i as f64 + 1.0).sin().abs() * 4294967296.0) as u32);
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend((data.len() as u64).wrapping_mul(8).to_le_bytes());
    for chunk in message.as_chunks::<64>().0 {
        let words = std::array::from_fn::<u32, 16, _>(|i| {
            u32::from_le_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap())
        });
        let [mut a, mut b, mut c, mut d] = state;
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let rotated =
                a.wrapping_add(f).wrapping_add(k[i]).wrapping_add(words[g]).rotate_left(S[i]);
            (a, d, c, b) = (d, c, b, b.wrapping_add(rotated));
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
    }
    let mut digest = [0; 16];
    for (i, word) in state.iter().enumerate() {
        digest[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    digest
}

/// The encoded filenames: the working directory, then each name, as
/// `CoverageFilenamesSectionWriter` writes them without compression.
fn encode_filenames(names: &[&str]) -> Vec<u8> {
    let mut strings = Vec::new();
    for name in names {
        uleb(&mut strings, name.len() as u64);
        strings.extend(name.as_bytes());
    }
    let mut out = Vec::new();
    uleb(&mut out, names.len() as u64);
    uleb(&mut out, strings.len() as u64);
    uleb(&mut out, 0);
    out.extend(strings);
    out
}

/// A counter reference as LLVM encodes it: the tag in the low two bits, then the ID.
fn encode_counter(expressions: &[Expression], counter: Counter) -> Option<u64> {
    Some(match counter {
        Counter::Zero => 0,
        Counter::Counter(id) => u64::from(id) << 2 | 1,
        Counter::Expression(id) => {
            let expression = expressions.get(id as usize)?;
            u64::from(id) << 2 | if expression.subtract { 2 } else { 3 }
        }
    })
}

/// Encodes the mapping of a function: its file table, expressions, and regions.
fn encode_mapping(
    function: &FunctionInput,
    file_ids: &BTreeMap<&str, u64>,
) -> eyre::Result<Vec<u8>> {
    let bad =
        || eyre::eyre!("`{}` has a counter that refers to a missing expression", function.name);
    let mut out = Vec::new();
    uleb(&mut out, function.filenames.len() as u64);
    for name in &function.filenames {
        uleb(&mut out, file_ids[name.as_str()]);
    }

    let expressions = &function.expressions;
    uleb(&mut out, expressions.len() as u64);
    for expression in expressions {
        uleb(&mut out, encode_counter(expressions, expression.lhs).ok_or_else(bad)?);
        uleb(&mut out, encode_counter(expressions, expression.rhs).ok_or_else(bad)?);
    }

    let mut index = 0;
    for file in 0..function.filenames.len() as u32 {
        let count = function.regions[index..].iter().take_while(|r| r.file == file).count();
        uleb(&mut out, count as u64);
        let mut previous_line = 0;
        for region in &function.regions[index..index + count] {
            let counter = |counter| encode_counter(expressions, counter).ok_or_else(bad);
            let kind = region.kind as u64;
            match region.kind {
                RegionKind::Code | RegionKind::Gap => uleb(&mut out, counter(region.count)?),
                RegionKind::Expansion => {
                    uleb(&mut out, 1 << 2 | u64::from(region.expanded_file) << 3);
                }
                RegionKind::Skipped => uleb(&mut out, kind << 3),
                RegionKind::Branch => {
                    uleb(&mut out, kind << 3);
                    uleb(&mut out, counter(region.count)?);
                    uleb(&mut out, counter(region.false_count)?);
                }
                RegionKind::McdcBranch => {
                    uleb(&mut out, kind << 3);
                    uleb(&mut out, counter(region.count)?);
                    uleb(&mut out, counter(region.false_count)?);
                    if let McdcParams::Branch { id, false_next, true_next } = region.mcdc {
                        // The IDs are written as their values plus one.
                        uleb(&mut out, (id + 1) as u64);
                        uleb(&mut out, (true_next + 1) as u64);
                        uleb(&mut out, (false_next + 1) as u64);
                    }
                }
                RegionKind::McdcDecision => {
                    uleb(&mut out, kind << 3);
                    if let McdcParams::Decision { bitmap_idx, num_conditions } = region.mcdc {
                        uleb(&mut out, bitmap_idx.into());
                        uleb(&mut out, num_conditions.into());
                    }
                }
            }
            uleb(&mut out, u64::from(region.start.0 - previous_line));
            uleb(&mut out, region.start.1.into());
            uleb(&mut out, u64::from(region.end.0 - region.start.0));
            // The high bit of a gap region's end column marks it.
            let gap_bit = if region.kind == RegionKind::Gap { 1 << 31 } else { 0 };
            uleb(&mut out, u64::from(region.end.1 | gap_bit));
            previous_line = region.start.0;
        }
        index += count;
    }
    Ok(out)
}

fn pad_to_8(out: &mut Vec<u8>) {
    out.resize(out.len().next_multiple_of(8), 0);
}

/// Writes an ELF64 relocatable object with the coverage mapping sections.
fn write_object(functions: &[FunctionInput]) -> eyre::Result<Vec<u8>> {
    // File IDs start at 1: ID 0 is the working directory, which is empty so that the names stay
    // relative to where `llvm-cov` runs.
    let mut file_ids = BTreeMap::new();
    for name in functions.iter().flat_map(|function| &function.filenames) {
        file_ids.entry(name.as_str()).or_insert(0);
    }
    for (id, value) in file_ids.values_mut().enumerate() {
        *value = id as u64 + 1;
    }
    let names = std::iter::once("").chain(file_ids.keys().copied()).collect::<Vec<_>>();
    let filenames = encode_filenames(&names);
    let filenames_ref = md5_low(&filenames);

    // `__llvm_covmap`: the header and the encoded filenames. The function records live in
    // `__llvm_covfun`, so the header counts none.
    let mut covmap = Vec::new();
    for value in [0, filenames.len() as u32, 0, COVMAP_VERSION] {
        covmap.extend(value.to_le_bytes());
    }
    covmap.extend(&filenames);
    pad_to_8(&mut covmap);

    // `__llvm_covfun`: one record per function, each aligned to 8 bytes.
    let mut covfun = Vec::new();
    for function in functions {
        let mapping = encode_mapping(function, &file_ids)?;
        covfun.extend(md5_low(function.name.as_bytes()).to_le_bytes());
        covfun.extend((mapping.len() as u32).to_le_bytes());
        covfun.extend(function.hash.to_le_bytes());
        covfun.extend(filenames_ref.to_le_bytes());
        covfun.extend(mapping);
        pad_to_8(&mut covfun);
    }

    // `__llvm_prf_names`: the uncompressed, separated names.
    let mut joined = String::new();
    for (index, function) in functions.iter().enumerate() {
        if index > 0 {
            joined.push(NAME_SEPARATOR);
        }
        joined.push_str(&function.name);
    }
    let mut prf_names = Vec::new();
    uleb(&mut prf_names, joined.len() as u64);
    uleb(&mut prf_names, 0);
    prf_names.extend(joined.as_bytes());

    Ok(elf_object(&[
        ("__llvm_covmap", covmap),
        ("__llvm_covfun", covfun),
        ("__llvm_prf_names", prf_names),
    ]))
}

/// Builds an ELF64 little-endian relocatable object holding allocatable data sections.
fn elf_object(sections: &[(&str, Vec<u8>)]) -> Vec<u8> {
    const EHSIZE: usize = 64;
    const SHENTSIZE: usize = 64;

    let mut shstrtab = vec![0u8];
    let mut name_offsets = Vec::new();
    for name in sections.iter().map(|(name, _)| *name).chain([".shstrtab"]) {
        name_offsets.push(shstrtab.len() as u32);
        shstrtab.extend(name.as_bytes());
        shstrtab.push(0);
    }

    let mut out = vec![0u8; EHSIZE];
    // (name offset, type, flags, file offset, size, alignment) of each section after the null one.
    let mut headers = Vec::new();
    for (index, (_, data)) in sections.iter().enumerate() {
        out.resize(out.len().next_multiple_of(8), 0);
        // SHT_PROGBITS, SHF_ALLOC.
        headers.push((name_offsets[index], 1u32, 2u64, out.len() as u64, data.len() as u64, 8u64));
        out.extend(data);
    }
    // SHT_STRTAB.
    headers.push((name_offsets[sections.len()], 3, 0, out.len() as u64, shstrtab.len() as u64, 1));
    out.extend(&shstrtab);
    out.resize(out.len().next_multiple_of(8), 0);
    let shoff = out.len() as u64;
    out.extend([0u8; SHENTSIZE]);
    for (name, kind, flags, offset, size, align) in headers {
        out.extend(name.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(flags.to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend(offset.to_le_bytes());
        out.extend(size.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(0u32.to_le_bytes());
        out.extend(align.to_le_bytes());
        out.extend(0u64.to_le_bytes());
    }

    let mut header = Vec::with_capacity(EHSIZE);
    header.extend([0x7f, b'E', b'L', b'F', 2, 1, 1, 0]);
    header.extend([0u8; 8]);
    header.extend(1u16.to_le_bytes()); // ET_REL.
    header.extend(62u16.to_le_bytes()); // EM_X86_64.
    header.extend(1u32.to_le_bytes());
    header.extend(0u64.to_le_bytes()); // Entry.
    header.extend(0u64.to_le_bytes()); // Program headers.
    header.extend(shoff.to_le_bytes());
    header.extend(0u32.to_le_bytes()); // Flags.
    header.extend((EHSIZE as u16).to_le_bytes());
    header.extend(0u16.to_le_bytes());
    header.extend(0u16.to_le_bytes());
    header.extend((SHENTSIZE as u16).to_le_bytes());
    header.extend((sections.len() as u16 + 2).to_le_bytes());
    header.extend((sections.len() as u16 + 1).to_le_bytes()); // `.shstrtab` index.
    out[..EHSIZE].copy_from_slice(&header);
    out
}

/// Writes the text instrumentation profile of the functions.
fn write_proftext(functions: &[FunctionInput]) -> eyre::Result<String> {
    let mut out = String::from(":fe\n");
    for function in functions {
        let counts = if function.counts.is_empty() { &[0][..] } else { &function.counts[..] };
        writeln!(
            out,
            "{}\n# Func Hash:\n{}\n# Num Counters:\n{}\n# Counter Values:",
            function.name,
            function.hash,
            counts.len()
        )
        .unwrap();
        for count in counts {
            writeln!(out, "{count}").unwrap();
        }
        if !function.bitmap.is_empty() {
            if function.bitmap.len() > 255 {
                eyre::bail!(
                    "the MC/DC bitmap of `{}` has {} bytes, but a text profile holds at most 255",
                    function.name,
                    function.bitmap.len()
                );
            }
            writeln!(out, "# Num Bitmap Bytes:\n${}\n# Bitmap Byte Values:", function.bitmap.len())
                .unwrap();
            for byte in &function.bitmap {
                writeln!(out, "0x{byte:x}").unwrap();
            }
        }
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_matches_the_reference_digests() {
        let hex = |data: &[u8]| md5(data).iter().map(|b| format!("{b:02x}")).collect::<String>();
        assert_eq!(hex(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            hex(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            ),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
        // The low half is the first eight digest bytes, read as a little-endian integer.
        assert_eq!(md5_low(b"abc"), 0xb04fd23c98500190);
    }

    #[test]
    fn leb128_matches_the_dwarf_examples() {
        let encode = |value| {
            let mut out = Vec::new();
            uleb(&mut out, value);
            out
        };
        assert_eq!(encode(2), [2]);
        assert_eq!(encode(127), [127]);
        assert_eq!(encode(128), [0x80, 1]);
        assert_eq!(encode(624485), [0xe5, 0x8e, 0x26]);
    }
}
