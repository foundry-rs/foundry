use crate::NumExt;
use std::{
	borrow::Cow,
	ffi::{OsStr, OsString},
	io,
	num::Saturating,
	os::windows::ffi::OsStrExt,
	path::{Path, PathBuf},
};
use widestring::{
	error::{ContainsNul, NulError},
	U16CStr, U16CString,
};

/// Conversion to WTF-16, the native string encoding of Windows NT.
pub trait ToWtf16<'a>: Sized {
	/// Encode to, or borrow as, WTF-16.
	///
	/// Borrowed string types may entail allocation and thus return [`Cow::Owned`] if an in-place
	/// checked cast fails.
	///
	/// # Errors
	/// If there are interior nuls.
	fn to_wtf_16(self) -> Result<Cow<'a, U16CStr>, ContainsNul<u16>>;
}

pub(crate) static EXPECT_WTF16: &str = "failed to convert to WTF-16";

pub(crate) fn to_io_error(err: ContainsNul<u16>) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidInput, err)
}

/// Trivial and infallible.
impl<'enc> ToWtf16<'enc> for &'enc U16CStr {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		Ok(Cow::Borrowed(self))
	}
}
/// Trivial and infallible.
impl<'enc> ToWtf16<'enc> for U16CString {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		Ok(Cow::Owned(self))
	}
}

/// Will allocate if the slice isn't nul-terminated.
impl<'enc> ToWtf16<'enc> for &'enc [u16] {
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		match U16CStr::from_slice(self) {
			Ok(borrow) => Ok(Cow::Borrowed(borrow)),
			Err(NulError::MissingNulTerminator(..)) => Ok(self.to_owned().to_wtf_16()?),
			Err(NulError::ContainsNul(cn)) => Err(cn),
		}
	}
}
/// Will `.push(0)` if the slice isn't nul-terminated, which may entail a memory allocation if the
/// `Vec` is at capacity.
impl<'enc> ToWtf16<'enc> for Vec<u16> {
	fn to_wtf_16(mut self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		if self.last() != Some(&0) {
			self.push(0);
		}
		Ok(Cow::Owned(U16CString::from_vec(self)?))
	}
}

/// Always reallocates, because `OsStr` is WTF-8.
impl<'enc, 'src> ToWtf16<'enc> for &'src OsStr {
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		Ok(Cow::Owned(U16CString::from_os_str(self)?))
	}
}
/// Always reallocates, because `OsString` is WTF-8.
impl<'enc> ToWtf16<'enc> for OsString {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		self.as_os_str().to_wtf_16()
	}
}

/// Always reallocates, because `Path` is WTF-8.
impl<'enc, 'src> ToWtf16<'enc> for &'src Path {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		self.as_os_str().to_wtf_16()
	}
}
/// Always reallocates, because `PathBuf` is WTF-8.
impl<'enc> ToWtf16<'enc> for PathBuf {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		self.into_os_string().to_wtf_16()
	}
}

/// Always reallocates, because `str` is UTF-8.
impl<'enc, 'src> ToWtf16<'enc> for &'src str {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		Ok(Cow::Owned(U16CString::from_str(self)?))
	}
}
/// Always reallocates, because `String` is UTF-8.
impl<'enc> ToWtf16<'enc> for String {
	#[inline]
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		self.as_str().to_wtf_16()
	}
}

impl<'enc, T: ?Sized, O> ToWtf16<'enc> for Cow<'enc, T>
where
	T: ToOwned<Owned = O>,
	&'enc T: ToWtf16<'enc>,
	O: ToWtf16<'enc>,
{
	fn to_wtf_16(self) -> Result<Cow<'enc, U16CStr>, ContainsNul<u16>> {
		match self {
			Cow::Borrowed(b) => b.to_wtf_16(),
			Cow::Owned(o) => o.to_wtf_16(),
		}
	}
}

fn pathcvt<'a>(
	pipe_name: &'a OsStr,
	hostname: Option<&'a OsStr>,
) -> (impl Iterator<Item = &'a OsStr>, usize) {
	const PREFIX_LITERAL: &str = r"\\";
	const PIPEFS_LITERAL: &str = r"\pipe\";
	const LOCAL_HOSTNAME: &str = ".";
	const BASE_LEN: Saturating<usize> = Saturating(PREFIX_LITERAL.len() + PIPEFS_LITERAL.len());

	let hostname = hostname.unwrap_or_else(|| OsStr::new(LOCAL_HOSTNAME));

	let components = [
		OsStr::new(PREFIX_LITERAL),
		hostname,
		OsStr::new(PIPEFS_LITERAL),
		pipe_name,
	];
	let userlen = hostname.len().saturate() + pipe_name.len().saturate();
	(components.into_iter(), (BASE_LEN + userlen).0)
}
pub(crate) fn convert_and_encode_path(
	pipename: &OsStr,
	hostname: Option<&OsStr>,
) -> io::Result<U16CString> {
	let (i, cap) = pathcvt(pipename, hostname);
	let mut path = Vec::with_capacity((cap.saturate() + 1.saturate()).0);
	i.for_each(|c| path.extend(c.encode_wide()));
	path.push(0); // Don't forget the nul terminator!
	U16CString::from_vec(path).map_err(contains_nul_error_to_io)
}

pub(crate) fn convert_osstr(str: &OsStr) -> io::Result<U16CString> {
	U16CString::from_os_str(str).map_err(contains_nul_error_to_io)
}

fn contains_nul_error_to_io(e: ContainsNul<u16>) -> io::Error {
	io::Error::new(
		io::ErrorKind::InvalidInput,
		format!("invalid named pipe path: {}", e),
	)
}
