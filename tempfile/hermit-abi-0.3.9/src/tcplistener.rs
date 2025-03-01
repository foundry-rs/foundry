//! `tcplistener` provide an interface to establish tcp socket server.

use crate::{Handle, IpAddress};

extern "Rust" {
	fn sys_tcp_listener_accept(port: u16) -> Result<(Handle, IpAddress, u16), ()>;
}

/// Wait for connection at specified address.
#[deprecated(since = "0.3.0", note = "please use new BSD socket interface")]
pub fn accept(port: u16) -> Result<(Handle, IpAddress, u16), ()> {
	unsafe { sys_tcp_listener_accept(port) }
}
