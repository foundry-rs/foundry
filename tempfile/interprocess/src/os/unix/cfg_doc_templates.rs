/*
// You can't generate those with macros just yet, so copypasting is the way for now.

#[cfg_attr( // uds_ucred template
	feature = "doc_cfg",
	doc(cfg(any(
		target_os = "linux",
		target_os = "redox",
		target_os = "android",
		target_os = "fuchsia",
	)))
)]

#[cfg_attr( // uds_cmsgcred template
	feature = "doc_cfg",
	doc(cfg(any(
		target_os = "freebsd",
		target_os = "dragonfly",
	)))
)]

#[cfg_attr( // uds_credentials template
	feature = "doc_cfg",
	doc(cfg(any(
		target_os = "linux",
		target_os = "redox",
		target_os = "android",
		target_os = "fuchsia",
		target_os = "freebsd",
		target_os = "dragonfly",
		target_os = "freebsd",
		target_os = "openbsd",
		target_os = "netbsd",
		target_os = "dragonfly",
		target_os = "macos",
		target_os = "ios",
		target_os = "tvos",
		target_os = "watchos",
	)))
)]
#[cfg_attr( // uds_ancillary_credentials template
	feature = "doc_cfg",
	doc(cfg(any(
		target_os = "linux",
		target_os = "redox",
		target_os = "android",
		target_os = "fuchsia",
		target_os = "freebsd",
		target_os = "dragonfly",
	)))
)]

#[cfg_attr( // uds_cont_credentials template
	feature = "doc_cfg",
	doc(cfg(any(
		target_os = "linux",
		target_os = "redox",
		target_os = "android",
		target_os = "fuchsia",
		target_os = "freebsd",
	)))
)]

#[cfg_attr( // uds_sockcred template
	feature = "doc_cfg",
	doc(cfg(target_os = "netbsd"))
)]

#[cfg_attr( // uds_sockcred2 template
	feature = "doc_cfg",
	doc(cfg(target_os = "freebsd"))
)]

#[cfg_attr( // uds_linux_namespace template
	feature = "doc_cfg",
	doc(cfg(any(target_os = "linux", target_os = "android")))
)]

*/
