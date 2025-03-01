use super::super::uds_local_socket::tokio as uds_impl;
use crate::local_socket::{
	tokio::{prelude::*, Listener, Stream},
	ListenerOptions, Name,
};
use std::io;

#[inline]
pub fn from_options(options: ListenerOptions<'_>) -> io::Result<Listener> {
	options
		.create_tokio_as::<uds_impl::Listener>()
		.map(Listener::from)
}

#[inline]
pub async fn connect(name: Name<'_>) -> io::Result<Stream> {
	uds_impl::Stream::connect(name).await.map(Stream::from)
}
