#[path = "util/mod.rs"]
#[macro_use]
mod util;

mod os {
	#[cfg(unix)]
	mod unix {
		mod local_socket_fake_ns;
		mod local_socket_mode;
	}
	#[cfg(windows)]
	mod windows {
		mod local_socket_security_descriptor;
		mod named_pipe;
		mod tokio_named_pipe;
	}
}

mod local_socket;
#[cfg(feature = "tokio")]
mod tokio_local_socket;

#[cfg(feature = "tokio")]
mod tokio_unnamed_pipe;
mod unnamed_pipe;
