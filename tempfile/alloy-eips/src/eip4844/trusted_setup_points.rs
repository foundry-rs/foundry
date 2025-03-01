use alloc::boxed::Box;
use alloy_primitives::hex;
use core::fmt;
use derive_more::{AsMut, AsRef, Deref, DerefMut};

pub use c_kzg::{BYTES_PER_G1_POINT, BYTES_PER_G2_POINT};

/// Number of G1 Points.
pub const NUM_G1_POINTS: usize = 4096;

/// Number of G2 Points.
pub const NUM_G2_POINTS: usize = 65;

/// A newtype over list of G1 point from kzg trusted setup.
#[derive(Clone, Debug, PartialEq, Eq, AsRef, AsMut, Deref, DerefMut)]
#[repr(transparent)]
pub struct G1Points(pub [[u8; BYTES_PER_G1_POINT]; NUM_G1_POINTS]);

impl Default for G1Points {
    fn default() -> Self {
        Self([[0; BYTES_PER_G1_POINT]; NUM_G1_POINTS])
    }
}

/// A newtype over list of G2 point from kzg trusted setup.
#[derive(Clone, Debug, PartialEq, Eq, AsRef, AsMut, Deref, DerefMut)]
#[repr(transparent)]
pub struct G2Points(pub [[u8; BYTES_PER_G2_POINT]; NUM_G2_POINTS]);

impl Default for G2Points {
    fn default() -> Self {
        Self([[0; BYTES_PER_G2_POINT]; NUM_G2_POINTS])
    }
}

/// Default G1 points.
pub const G1_POINTS: &G1Points = {
    const BYTES: &[u8] = include_bytes!("./g1_points.bin");
    assert!(BYTES.len() == core::mem::size_of::<G1Points>());
    unsafe { &*BYTES.as_ptr().cast::<G1Points>() }
};

/// Default G2 points.
pub const G2_POINTS: &G2Points = {
    const BYTES: &[u8] = include_bytes!("./g2_points.bin");
    assert!(BYTES.len() == core::mem::size_of::<G2Points>());
    unsafe { &*BYTES.as_ptr().cast::<G2Points>() }
};

/// Parses the contents of a KZG trusted setup file into a list of G1 and G2 points.
///
/// These can then be used to create a KZG settings object with
/// [`KzgSettings::load_trusted_setup`](c_kzg::KzgSettings::load_trusted_setup).
pub fn parse_kzg_trusted_setup(
    trusted_setup: &str,
) -> Result<(Box<G1Points>, Box<G2Points>), KzgErrors> {
    let mut lines = trusted_setup.lines();

    // load number of points
    let n_g1 = lines
        .next()
        .ok_or(KzgErrors::FileFormatError)?
        .parse::<usize>()
        .map_err(|_| KzgErrors::ParseError)?;
    let n_g2 = lines
        .next()
        .ok_or(KzgErrors::FileFormatError)?
        .parse::<usize>()
        .map_err(|_| KzgErrors::ParseError)?;

    if n_g1 != NUM_G1_POINTS {
        return Err(KzgErrors::MismatchedNumberOfPoints);
    }

    if n_g2 != NUM_G2_POINTS {
        return Err(KzgErrors::MismatchedNumberOfPoints);
    }

    // load g1 points
    let mut g1_points = Box::<G1Points>::default();
    for bytes in &mut g1_points.0 {
        let line = lines.next().ok_or(KzgErrors::FileFormatError)?;
        hex::decode_to_slice(line, bytes).map_err(|_| KzgErrors::ParseError)?;
    }

    // load g2 points
    let mut g2_points = Box::<G2Points>::default();
    for bytes in &mut g2_points.0 {
        let line = lines.next().ok_or(KzgErrors::FileFormatError)?;
        hex::decode_to_slice(line, bytes).map_err(|_| KzgErrors::ParseError)?;
    }

    if lines.next().is_some() {
        return Err(KzgErrors::FileFormatError);
    }

    Ok((g1_points, g2_points))
}

/// KZG custom Error types
#[derive(Clone, Copy, Debug)]
pub enum KzgErrors {
    /// Failed to get current directory.
    FailedCurrentDirectory,
    /// The specified path does not exist.
    PathNotExists,
    /// Problems related to I/O.
    IOError,
    /// Not a valid file.
    NotValidFile,
    /// File is not properly formatted.
    FileFormatError,
    /// Not able to parse to usize.
    ParseError,
    /// Number of points does not match what is expected.
    MismatchedNumberOfPoints,
}

impl fmt::Display for KzgErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::FailedCurrentDirectory => "failed to get current directory",
            Self::PathNotExists => "the specified path does not exist",
            Self::IOError => "IO error",
            Self::NotValidFile => "not a valid file",
            Self::FileFormatError => "file is not properly formatted",
            Self::ParseError => "could not parse as usize",
            Self::MismatchedNumberOfPoints => "number of points does not match what is expected",
        };
        f.write_str(s)
    }
}

impl core::error::Error for KzgErrors {}
