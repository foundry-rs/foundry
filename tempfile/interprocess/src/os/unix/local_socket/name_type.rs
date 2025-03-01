use crate::local_socket::{Name, NameInner, NameType, NamespacedNameType, PathNameType};
use std::{
	borrow::Cow,
	ffi::{CStr, OsStr, OsString},
	io,
	os::unix::prelude::*,
};

fn c2os(ccow: Cow<'_, CStr>) -> Cow<'_, OsStr> {
	match ccow {
		Cow::Borrowed(cstr) => Cow::Borrowed(OsStr::from_bytes(cstr.to_bytes())),
		Cow::Owned(cstring) => Cow::Owned(OsString::from_vec(cstring.into_bytes())),
	}
}

tag_enum!(
/// [Mapping](NameType) that produces local socket names referring to Unix domain sockets bound to
/// the filesystem.
///
/// For Unix domain sockets residing in the Linux abstract namespace, see `AbstractNsUdSocket`
/// instead.
FilesystemUdSocket);
impl NameType for FilesystemUdSocket {
	fn is_supported() -> bool {
		true
	}
}
impl PathNameType<OsStr> for FilesystemUdSocket {
	#[inline]
	fn map(path: Cow<'_, OsStr>) -> io::Result<Name<'_>> {
		for b in path.as_bytes() {
			if *b == 0 {
				return Err(io::Error::new(
					io::ErrorKind::InvalidInput,
					"filesystem paths cannot contain interior nuls",
				));
			}
		}
		Ok(Name(NameInner::UdSocketPath(path)))
	}
}
impl PathNameType<CStr> for FilesystemUdSocket {
	#[inline]
	fn map(path: Cow<'_, CStr>) -> io::Result<Name<'_>> {
		Self::map(c2os(path))
	}
}

tag_enum!(
/// [Mapping](NameType) that produces local socket names referring to Unix domain sockets bound to
/// special locations in the filesystems that are interpreted as dedicated namespaces.
///
/// This is the substitute for `AbstractNsUdSocket` on non-Linux Unices, and is the only available
/// [namespaced name type](NamespacedNameType) on those systems.
SpecialDirUdSocket);
impl NameType for SpecialDirUdSocket {
	fn is_supported() -> bool {
		true
	}
}
impl NamespacedNameType<OsStr> for SpecialDirUdSocket {
	#[inline]
	fn map(name: Cow<'_, OsStr>) -> io::Result<Name<'_>> {
		for b in name.as_bytes() {
			if *b == 0 {
				return Err(io::Error::new(
					io::ErrorKind::InvalidInput,
					"special directory-bound names cannot contain interior nuls",
				));
			}
		}
		Ok(Name(NameInner::UdSocketPseudoNs(name)))
	}
}
impl NamespacedNameType<CStr> for SpecialDirUdSocket {
	#[inline]
	fn map(name: Cow<'_, CStr>) -> io::Result<Name<'_>> {
		Self::map(c2os(name))
	}
}

#[cfg(any(target_os = "linux", target_os = "android"))]
tag_enum!(
/// [Mapping](NameType) that produces local socket names referring to Unix domain sockets bound to
/// the Linux abstract namespace.
#[cfg_attr(feature = "doc_cfg", doc(cfg(any(target_os = "linux", target_os = "android"))))]
AbstractNsUdSocket);
#[cfg(any(target_os = "linux", target_os = "android"))]
impl NameType for AbstractNsUdSocket {
	fn is_supported() -> bool {
		true // Rust is unsupported on Linux below version 3.2
	}
}
#[cfg(any(target_os = "linux", target_os = "android"))]
impl NamespacedNameType<OsStr> for AbstractNsUdSocket {
	#[inline]
	fn map(name: Cow<'_, OsStr>) -> io::Result<Name<'_>> {
		let name = match name {
			Cow::Borrowed(b) => Cow::Borrowed(b.as_bytes()),
			Cow::Owned(o) => Cow::Owned(o.into_vec()),
		};
		Ok(Name(NameInner::UdSocketNs(name)))
	}
}
#[cfg(any(target_os = "linux", target_os = "android"))]
impl NamespacedNameType<CStr> for AbstractNsUdSocket {
	#[inline]
	fn map(name: Cow<'_, CStr>) -> io::Result<Name<'_>> {
		Self::map(c2os(name))
	}
}

macro_rules! map_generic {
	(path $name:ident for $str:ident) => {
		pub(crate) fn $name(path: Cow<'_, $str>) -> io::Result<Name<'_>> {
			FilesystemUdSocket::map(path)
		}
	};
	(namespaced $name:ident for $str:ident) => {
		pub(crate) fn $name(name: Cow<'_, $str>) -> io::Result<Name<'_>> {
			#[cfg(any(target_os = "linux", target_os = "android"))]
			{
				AbstractNsUdSocket::map(name)
			}
			#[cfg(not(any(target_os = "linux", target_os = "android")))]
			{
				SpecialDirUdSocket::map(name)
			}
		}
	};
	($($type:ident $name:ident for $str:ident)+) => {$(
		map_generic!($type $name for $str);
	)+};
}

map_generic! {
	path		map_generic_path_osstr			for OsStr
	path		map_generic_path_cstr			for CStr
	namespaced	map_generic_namespaced_osstr	for OsStr
	namespaced	map_generic_namespaced_cstr		for CStr
}
