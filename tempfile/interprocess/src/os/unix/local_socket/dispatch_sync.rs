use super::super::uds_local_socket as uds_impl;
use crate::local_socket::{prelude::*, Listener, ListenerOptions, Name, Stream};
use std::io;

#[inline]
pub fn from_options(options: ListenerOptions<'_>) -> io::Result<Listener> {
	options
		.create_sync_as::<uds_impl::Listener>()
		.map(Listener::from)
}

#[inline]
pub fn connect(name: Name<'_>) -> io::Result<Stream> {
	uds_impl::Stream::connect(name).map(Stream::from)
}
