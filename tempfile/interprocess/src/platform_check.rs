#[cfg(any(not(any(windows, unix)), target_os = "emscripten"))]
compile_error!(
	"Your target operating system is not supported by interprocess – check if yours is in the list \
of supported systems, and if not, please open an issue on the GitHub repository if you think that \
it should be included"
);

#[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
compile_error!(
	"Platforms with exotic pointer widths (neither 32-bit nor 64-bit) are not supported by \
interprocess – if you think that your specific case needs to be accounted for, please open an \
issue on the GitHub repository"
);
