//! Local sockets, an IPC primitive featuring a server and multiple clients connecting to that
//! server using a filesystem path or an identifier inside a special namespace, each having a
//! private connection to that server.
//!
//! ## Implementation types
//! Local sockets are not a real IPC method implemented by the OS – they exist to paper over the
//! differences between the two underlying implementations currently in use: Unix domain sockets and
//! Windows named pipes.
//!
//! Interprocess defines [traits] that implementations of local sockets implement, and enums that
//! constitute devirtualized trait objects (not unlike those provided by the `enum_dispatch` crate)
//! for those traits. The implementation used, in cases where multiple options apply, is chosen at
//! construction via the [name](Name) and [name type](NameType) infrastructure.
//!
//! ## Differences from regular sockets
//! A few missing features, primarily on Windows, require local sockets to omit some important
//! functionality, because code relying on it wouldn't be portable. Some notable differences are:
//! -	No `.shutdown()` – your communication protocol must manually negotiate end of transmission.
//! 	Notably, `.read_to_string()` and `.read_all()` will always block indefinitely at some point.
//! -	No datagram sockets – the difference in semantics between connectionless datagram Unix-domain
//! 	sockets and connection-based named message pipes on Windows does not allow bridging those two
//! 	into a common API. You can emulate datagrams on top of streams anyway, so no big deal, right?

#[macro_use]
mod enumdef;

mod name;
mod stream {
	pub(super) mod r#enum;
	pub(super) mod r#trait;
}
mod listener {
	pub(super) mod r#enum;
	pub(super) mod options;
	pub(super) mod r#trait;
}

/// Traits representing the interface of local sockets.
pub mod traits {
	pub use super::{
		listener::r#trait::{Listener, ListenerExt, ListenerNonblockingMode},
		stream::r#trait::*,
	};
	/// Traits for the Tokio variants of local socket objects.
	#[cfg(feature = "tokio")]
	#[cfg_attr(feature = "doc_cfg", doc(cfg(feature = "tokio")))]
	pub mod tokio {
		pub use super::super::tokio::{listener::r#trait::*, stream::r#trait::*};
	}
}

pub use {
	listener::{options::ListenerOptions, r#enum::*, r#trait::Incoming},
	name::*,
	stream::r#enum::*,
	traits::ListenerNonblockingMode,
};

/// Re-exports of [traits] done in a way that doesn't pollute the scope, as well as of the
/// enum-dispatch types with their names prefixed with `LocalSocket`.
pub mod prelude {
	pub use super::{
		name::{NameType as _, ToFsName as _, ToNsName as _},
		traits::{Listener as _, ListenerExt as _, Stream as _},
		Listener as LocalSocketListener, Stream as LocalSocketStream,
	};
}

/// Asynchronous local sockets which work with the Tokio runtime and event loop.
///
/// The Tokio integration allows the local socket streams and listeners to be notified by the OS
/// kernel whenever they're ready to be received from of sent to, instead of spawning threads just
/// to put them in a wait state of blocking on the I/O.
///
/// Types from this module will *not* work with other async runtimes, such as `async-std` or `smol`,
/// since the Tokio types' methods will panic whenever they're called outside of a Tokio runtime
/// context. Open an issue if you'd like to see other runtimes supported as well.
#[cfg(feature = "tokio")]
#[cfg_attr(feature = "doc_cfg", doc(cfg(feature = "tokio")))]
pub mod tokio {
	pub(super) mod listener {
		pub(in super::super) mod r#enum;
		pub(in super::super) mod r#trait;
	}
	pub(super) mod stream {
		pub(in super::super) mod r#enum;
		pub(in super::super) mod r#trait;
	}
	pub use {listener::r#enum::*, stream::r#enum::*};

	/// Like the [sync local socket prelude](super::prelude), but for Tokio local sockets.
	pub mod prelude {
		pub use super::{
			super::{
				name::{NameType as _, ToFsName as _, ToNsName as _},
				traits::tokio::{Listener as _, Stream as _},
			},
			Listener as LocalSocketListener, Stream as LocalSocketStream,
		};
	}
}

mod concurrency_detector;
pub(crate) use concurrency_detector::*;
