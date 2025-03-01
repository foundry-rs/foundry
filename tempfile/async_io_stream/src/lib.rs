#![ cfg_attr( nightly, feature(doc_cfg) ) ]
#![ doc = include_str!("../README.md") ]

#![ doc    ( html_root_url = "https://docs.rs/async_io_stream" ) ]
#![ deny   ( missing_docs                                      ) ]
#![ forbid ( unsafe_code                                       ) ]
#![ allow  ( clippy::suspicious_else_formatting                ) ]

#![ warn
(
	anonymous_parameters          ,
	missing_copy_implementations  ,
	missing_debug_implementations ,
	nonstandard_style             ,
	rust_2018_idioms              ,
	trivial_casts                 ,
	trivial_numeric_casts         ,
	unreachable_pub               ,
	unused_extern_crates          ,
	unused_qualifications         ,
	variant_size_differences      ,
)]



// External dependencies
//
use
{
	std          :: { fmt, io::{ self, Read, Cursor, IoSlice, IoSliceMut, BufRead }                 } ,
	std          :: { pin::Pin, task::{ Poll, Context }, borrow::{ Borrow, BorrowMut }              } ,
	futures      :: { TryStream, ready, Sink, task::noop_waker, AsyncRead, AsyncWrite, AsyncBufRead } ,
};


#[ cfg( feature = "tokio_io" ) ]
//
use tokio::io::{ AsyncRead as TokAsyncRead, AsyncWrite as TokAsyncWrite };

#[ cfg( feature = "map_pharos" ) ]
//
use
{
	pharos::{ Observable, ObservableLocal, ObserveConfig, Observe, ObserveLocal },
	futures::FutureExt
};


// A buffer for the current message or error.
//
#[ derive(Debug) ]
//
enum ReadState<B>
{
	Ready{ chunk: Cursor<B> } ,
	Error{ error: io::Error } ,
	Eof                       ,
}


/// A wrapper over a TryStream + Sink that implements [`AsyncRead`]/[`AsyncWrite`] and [`AsyncBufRead`].
/// See the readme for more information.
///
/// Implements [`tokio::io::AsyncRead`] and [`tokio::io::AsyncWrite`] when the `tokio_io` feature is enabled.
//
pub struct IoStream<St, I>
where

	St: Unpin,
{
	inner    : St                   ,
	state    : Option<ReadState<I>> ,
	write_err: Option<io::Error>    ,
}

impl<St, I> Unpin for IoStream<St, I>
where

	St: Unpin,
{}


impl<St, I> IoStream<St, I>
where

	St: Unpin,

{
	/// Create a new IoStream.
	//
	pub fn new( inner: St ) -> Self
	{
		Self
		{
			inner            ,
			state     : None ,
			write_err : None ,
		}
	}


	/// Get a reference to the inner stream.
	//
	pub fn inner( &self ) -> &St
	{
		&self.inner
	}


	/// Get a mut reference to the inner stream.
	//
	pub fn inner_mut( &mut self ) -> &mut St
	{
		&mut self.inner
	}



	// The requirements:
	// - fill as much of the passed in buffer as we can.
	// - the item coming out of the stream might be bigger than the read buffer, so then we need
	//   to buffer it internally and keep track of how many bytes are left for next time.
	// - the item might be smaller than the buffer, but then we need to get the next item
	//   out of the stream. So the stream might return:
	//   - the next item,  hurray
	//   - pending
	//   - an error.
	//
	//   If it returns pending, we cannot return pending as we already have copied bytes into the
	//   output buffer, so we need to return Poll::Ready( Ok(n) ) where n is the number of bytes read.
	//
	//   However, the stream will wake up the waker that it got, but we didn't return pending here,
	//   so to avoid the spurious wakeups, we will just call that with a dummy waker if we already
	//   have data to return.
	//
	//   If it returns an error, we now need to buffer that error for the next call to poll_read,
	//   because again we can not return it immediately.
	//
	fn poll_read_impl( mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8] ) -> Poll< io::Result<usize> >
	where

		St: TryStream< Ok=I, Error=io::Error >,
		I: AsRef<[u8]>,

	{
		// since we might call the inner stream several times, keep track of whether we have data to
		// return. If we do, we cannot return pending or error, but need to buffer the error for next
		// call.
		//
		let mut have_read = 0;
		let mut state     = self.state.take();

		loop { match state
		{
			Some( ReadState::Eof ) => return Poll::Ready( Ok(0) ),

			// A buffered error from the last call to poll_read.
			//
			Some( ReadState::Error{ error } ) =>
			{
				self.state = None;
				return Poll::Ready( Err(error) )
			}

			Some( ReadState::Ready{ ref mut chunk } ) =>
			{
				have_read += chunk.read( &mut buf[have_read..] ).expect( "no io errors on cursor" );

				// We read the entire chunk
				//
				if chunk.position() == chunk.get_ref().as_ref().len() as u64
				{
					state = None;
				}


				// The buffer is full, we are done.
				//
				if have_read == buf.len()
				{
					self.state = state;
					return Poll::Ready( Ok(have_read) );
				}
			}


			None =>
			{
				if have_read == 0
				{
					match ready!( Pin::new( &mut self.inner ).try_poll_next( cx ) )
					{
						// We have an item. Store it and continue the loop.
						//
						Some(Ok( chunk )) =>
						{
							state = ReadState::Ready { chunk: Cursor::new(chunk) }.into();
						}

						// The stream has ended
						//
						None =>
						{
							self.state = ReadState::Eof.into();
							return Ok(0).into();
						}

						Some( Err(err) ) =>
						{
							// We didn't put anything in the passed in buffer, so just
							// return the error.
							//
							self.state = None;
							return Poll::Ready(Err( err ))
						}
					}
				}

				// there is already data ready to be returned, but the passed in buffer still
				// has space, so we try to get another item out of the stream.
				//
				else
				{
					// We won't be able to return pending as we already have data, so make sure
					// the stream doesn't try to wake up the task.
					//
					let     waker   = noop_waker();
					let mut context = Context::from_waker( &waker );

					match Pin::new( &mut self.inner ).try_poll_next( &mut context )
					{
						// We have an item. Store it and continue the loop.
						//
						Poll::Ready( Some(Ok( chunk )) ) =>

							state = ReadState::Ready { chunk: Cursor::new(chunk) }.into(),


						// The stream has ended
						//
						Poll::Ready( None ) =>
						{
							// return whatever we had already read.
							//
							self.state = ReadState::Eof.into();
							return Ok(have_read).into();
						}

						Poll::Ready(Some( Err(err) )) =>
						{
							self.state = ReadState::Error{ error: err }.into();
							return Ok(have_read).into();
						}

						Poll::Pending =>
						{
							self.state = None;
							return Ok(have_read).into();
						}
					}
				}
			}

		}} // loop, match
	}


	// Try to fill as many buffers as possible. Only go the next buffer if the current one is full.
	//
	// If poll_read returns Pending and we already have data, we can ignore it.
	// If poll_read returns an error and we already have data, we store it for the next call.
	//
	fn poll_read_vectored_impl( mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &mut [IoSliceMut<'_>] ) -> Poll< io::Result<usize> >
	where

		St: TryStream< Ok=I, Error=io::Error >,
		I: AsRef<[u8]>,

	{
		let mut have_read = 0;

		for b in bufs
		{
			if !b.is_empty()
			{
				// calling with the first buffer, either poll_read fills it completely, or we return.
				//
				if have_read == 0
				{
					match ready!( self.as_mut().poll_read_impl( cx, b ) )
					{
						// order matters
						//
						Err(e)                => return Poll::Ready( Err(e) ) ,
						Ok (n) if n < b.len() => return Poll::Ready( Ok (n) ) , // stream is pending or ended

						Ok(n) => // we filled entire buffer, we can pass to the next one
						{
							debug_assert!( n == b.len() );
							have_read += n;
						}
					}
				}

				else // have_read != 0, this not the first buffer
				{
					// We won't be able to return pending as we already have data, so make sure
					// the stream doesn't try to wake up the task.
					//
					let     waker   = noop_waker();
					let mut context = Context::from_waker( &waker );

					// either it fills the entire buffer, or we return.
					//
					match self.as_mut().poll_read_impl( &mut context, b )
					{
						// order matters
						//
						Poll::Pending                       => return Poll::Ready( Ok(have_read    ) ) ,
						Poll::Ready( Ok(n) ) if n < b.len() => return Poll::Ready( Ok(have_read + n) ) , // stream is pending or ended

						Poll::Ready( Ok(n) ) => // we filled entire buffer, we can pass to the next one
						{
							debug_assert!( n == b.len() );
							have_read += n;
						}

						Poll::Ready( Err(e) ) =>
						{
							// store the error for next time, because we have to return have_read first.
							//
							self.state = ReadState::Error{ error: e }.into();
							return Poll::Ready( Ok(have_read) );
						}
					}
				}
			}
		}

		// Either all buffers where zero length, or we filled all of them.
		// I'm not sure what the point is of polling the stream if we just get an empty buffer,
		// but it's what the default impls in std and futures do, so let's be consistent.
		//
		if   have_read == 0 { self.poll_read_impl( cx, &mut [] ) }
		else                { Poll::Ready( Ok(have_read) )       }
	}


	fn poll_write_impl<'a>( mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &'a [u8] ) -> Poll< io::Result<usize> >
	where

		St: Sink< I, Error=io::Error >,
		I: From< Vec<u8> >,

	{
		if let Some( e ) = self.write_err.take()
		{
			return Poll::Ready( Err(e) );
		}

		// Must call this before start_send
		//
		let res = ready!( Pin::new( &mut self.inner ).poll_ready(cx) );

		if let Err( e ) = res
		{
			return Poll::Ready( Err(e) );
		}


		match Pin::new( &mut self.inner ).start_send( buf.to_vec().into() )
		{
			Ok (_) =>
			{
				// Client code like futures-codec and tokio-codec turn a flush on their sink in a poll_write here.
				// Combinators like CopyBufInto will only call flush after their entire input
				// stream is exhausted. This is a problem if the source temporarily goes dry.
				// We actually don't buffer here, but always create an entire websocket message from the
				// buffer we get in poll_write, so there is no reason not to flush here.
				// This means the burden is on the caller to call with a buffer of sufficient size
				// to avoid perf problems, but there is BufReader and BufWriter in the futures library to
				// help with that if necessary.
				//
				// We will ignore the Pending return from the flush, since we took the data and
				// must return how many bytes we took. The client should not try to send this data again.
				// This does mean there might be a spurious wakeup, TODO: we should test that.
				// We could supply a dummy context to avoid the wakup.
				//
				// So, flush!
				//
				let     waker   = noop_waker();
				let mut context = Context::from_waker( &waker );

				match Pin::new( &mut self.inner ).poll_flush( &mut context )
				{
					Poll::Pending         | // ignore
					Poll::Ready( Ok(_) ) => {}

					Poll::Ready( Err(e)) => self.write_err = e.into(),
				}


				Poll::Ready(Ok( buf.len() ))
			}

			Err(e) => Poll::Ready( Err(e) ),
		}
	}


	fn poll_write_vectored_impl<'a>( mut self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &'a[ IoSlice<'a> ] ) -> Poll< io::Result<usize> >
	where

		St: Sink< I, Error=io::Error >,
		I: From< Vec<u8> >,

	{
		if let Some( e ) = self.write_err.take()
		{
			return Poll::Ready( Err(e) );
		}


		// Must call this before start_send
		//
		let res = ready!( Pin::new( &mut self.inner ).poll_ready(cx) );

		if let Err( e ) = res
		{
			return Poll::Ready( Err(e) )
		}


		let mut wrote = 0;

		for buf in bufs { wrote += buf.len(); }

		let mut item = Vec::with_capacity( wrote );

		for buf in bufs
		{
			item.extend_from_slice( buf );
		}


		match Pin::new( &mut self.inner ).start_send( item.into() )
		{
			Ok (_) =>
			{
				// Client code like futures-codec and tokio-codec turn a flush on their sink in a poll_write here.
				// Combinators like CopyBufInto will only call flush after their entire input
				// stream is exhausted. This is a problem if the source temporarily goes dry.
				// We actually don't buffer here, but always create an entire websocket message from the
				// buffer we get in poll_write, so there is no reason not to flush here.
				// This means the burden is on the caller to call with a buffer of sufficient size
				// to avoid perf problems, but there is BufReader and BufWriter in the futures library to
				// help with that if necessary.
				//
				// We will ignore the Pending return from the flush, since we took the data and
				// must return how many bytes we took. The client should not try to send this data again.
				// This does mean there might be a spurious wakeup, TODO: we should test that.
				// We could supply a dummy context to avoid the wakup.
				//
				// So, flush!
				//
				let     waker   = noop_waker();
				let mut context = Context::from_waker( &waker );

				match Pin::new( &mut self.inner ).poll_flush( &mut context )
				{
					Poll::Pending         | // ignore
					Poll::Ready( Ok(_) ) => {}

					Poll::Ready( Err(e)) => self.write_err = e.into(),
				}

				Poll::Ready(Ok( wrote ))
			}

			Err(e) => Poll::Ready( Err(e) ),
		}
	}



	fn poll_flush_impl(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll< io::Result<()> >
	where

		St: Sink< I, Error=io::Error >
	{
		match ready!( Pin::new( &mut self.inner ).poll_flush(cx) )
		{
			Ok (_) => Poll::Ready(Ok ( () )) ,
			Err(e) => Poll::Ready(Err( e  )) ,
		}
	}


	fn poll_close_impl( mut self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<()> >
	where

		St: Sink< I, Error=io::Error >
	{
		ready!( Pin::new( &mut self.inner ).poll_close( cx ) ).into()
	}
}



impl<St: Unpin, I> fmt::Debug for IoStream<St, I>
{
	fn fmt( &self, f: &mut fmt::Formatter<'_> ) -> fmt::Result
	{
		write!( f, "IoStream over Tungstenite" )
	}
}
/// ### Errors
///
/// The following errors can be returned when writing to the stream:
///
/// - [`io::ErrorKind::NotConnected`]: This means that the connection is already closed. You should
///   drop it. It is safe to drop the underlying connection.
///
/// - [`io::ErrorKind::InvalidData`]: This means that a tungstenite::error::Capacity occurred. This means that
///   you send in a buffer bigger than the maximum message size configured on the underlying websocket connection.
///   If you did not set it manually, the default for tungstenite is 64MB.
///
/// - other std::io::Error's generally mean something went wrong on the underlying transport. Consider these fatal
///   and just drop the connection.
//
impl<St, I> AsyncWrite for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + Unpin,
	I: From< Vec<u8> >

{
	/// Will always flush the underlying socket. Will always create an entire Websocket message from every write,
	/// so call with a sufficiently large buffer if you have performance problems.
	//
	fn poll_write( self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8] ) -> Poll< io::Result<usize> >
	{
		self.poll_write_impl( cx, buf )
	}


	fn poll_write_vectored( self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &[ IoSlice<'_> ] ) -> Poll< io::Result<usize> >
	{
		self.poll_write_vectored_impl( cx, bufs )
	}


	fn poll_flush( self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<()> >
	{
		self.poll_flush_impl( cx )
	}


	fn poll_close( self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<()> >
	{
		self.poll_close_impl( cx )
	}
}



#[ cfg( feature = "tokio_io" ) ]
//
#[ cfg_attr( nightly, doc(cfg( feature = "tokio_io" )) ) ]
//
impl<St, I> TokAsyncWrite for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + Unpin,
	I: From< Vec<u8> >

{
	fn poll_write( self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8] ) -> Poll< io::Result<usize> >
	{
		self.poll_write_impl( cx, buf )
	}


	fn poll_flush( self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<()> >
	{
		self.poll_flush_impl( cx )
	}


	fn poll_shutdown( self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<()> >
	{
		self.poll_close_impl( cx )
	}
}




impl<St, I> AsyncRead  for IoStream<St, I>
where

	St: TryStream< Ok=I, Error=io::Error > + Unpin,
	I: AsRef<[u8]>

{
	fn poll_read( self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8] ) -> Poll< io::Result<usize> >
	{
		self.poll_read_impl( cx, buf )
	}

	fn poll_read_vectored( self: Pin<&mut Self>, cx: &mut Context<'_>, bufs: &mut [IoSliceMut<'_>] ) -> Poll< io::Result<usize> >
	{
		self.poll_read_vectored_impl( cx, bufs )
	}
}


#[ cfg( feature = "tokio_io" ) ]
//
#[ cfg_attr( nightly, doc(cfg( feature = "tokio_io" )) ) ]
//
/// This impl requires the `tokio_io` feature.
//
impl<St, I> TokAsyncRead for IoStream<St, I>
where

	St: TryStream< Ok=I, Error=io::Error > + Unpin,
	I: AsRef<[u8]>

{
	fn poll_read( self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut tokio::io::ReadBuf<'_> ) -> Poll< io::Result<()> >
	{
		let read_count = ready!( self.poll_read_impl( cx, buf.initialize_unfilled() ) )?;
		buf.advance( read_count );

		Poll::Ready( Ok(()) )
	}
}




#[ cfg( feature = "map_pharos" ) ]
//
#[ cfg_attr( nightly, doc(cfg( feature = "map_pharos" )) ) ]
//
/// This impl requires the `map_pharos` feature.
//
impl<St, I, Ev> Observable<Ev> for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + TryStream< Ok=I, Error=io::Error > + Observable<Ev> + Unpin,
	Ev: Clone + Send + 'static,
	St: Send,
	I: Send,

{
	type Error = <St as Observable<Ev>>::Error;

	fn observe( &mut self, options: ObserveConfig<Ev> ) -> Observe< '_, Ev, Self::Error >
	{
		async move
		{
			let result = self.inner.observe( options ).await;

			result.map_err( Into::into )

		}.boxed()
	}
}




#[ cfg( feature = "map_pharos" ) ]
//
#[ cfg_attr( nightly, doc(cfg( feature = "map_pharos" )) ) ]
//
/// This impl requires the `map_pharos` feature.
//
impl<St, I, Ev> ObservableLocal<Ev> for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + TryStream< Ok=I, Error=io::Error > + ObservableLocal<Ev> + Unpin,
	Ev: Clone + Send + 'static,

{
	type Error = <St as ObservableLocal<Ev>>::Error;

	fn observe_local( &mut self, options: ObserveConfig<Ev> ) -> ObserveLocal< '_, Ev, Self::Error >
	{
		async move
		{
			let result = self.inner.observe_local( options ).await;

			result.map_err( Into::into )

		}.boxed_local()
	}
}



impl<St, I> Borrow<St> for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + TryStream< Ok=I, Error=io::Error > + Unpin,

{
	fn borrow( &self ) -> &St
	{
		&self.inner
	}
}



impl<St, I> BorrowMut<St> for IoStream<St, I>
where

	St: Sink< I, Error=io::Error > + TryStream< Ok=I, Error=io::Error > + Unpin,

{
	fn borrow_mut( &mut self ) -> &mut St
	{
		&mut self.inner
	}
}




impl<St, I> AsyncBufRead for IoStream<St, I>
where

	St: TryStream<Ok=I, Error = io::Error> + Unpin ,
	I : AsRef<[u8]> + Unpin                        ,

{
	fn poll_fill_buf( mut self: Pin<&mut Self>, cx: &mut Context<'_> ) -> Poll< io::Result<&[u8]> >
	{
		if self.state.is_none()
		{
			match ready!( Pin::new( &mut self.inner ).try_poll_next(cx) )
			{
				Some( Ok(chunk) ) =>
				{
					if !chunk.as_ref().is_empty()
					{
						self.state = ReadState::Ready
						{
							chunk: Cursor::new( chunk ),

						}.into();
					}
				}

				Some( Err(error) ) =>
				{
					self.state = ReadState::Error{ error }.into();
				}

				None =>
				{
					self.state = ReadState::Eof.into();
				}
			}
		}


		match self.state.take()
		{
			Some( ReadState::Error{ error } ) =>
			{
				self.state = None;
				Poll::Ready( Err(error) )
			}

			Some( ReadState::Eof ) =>
			{
				Poll::Ready( Ok(&[]) )
			}

			Some(x) =>
			{
				// Put it back, because we will return a reference to the buffer.
				//
				self.state = Some(x);

				if let Some( ReadState::Ready{ ref mut chunk } ) = self.get_mut().state
				{
					return Poll::Ready( chunk.fill_buf() );
				}

				unreachable!();
			}

			None => unreachable!(),
		}
	}



	fn consume( mut self: Pin<&mut Self>, amount: usize )
	{
		if amount == 0 { return }

		if let Some( ReadState::Ready{ chunk } ) = &mut self.state
		{
			chunk.consume( amount );

			// if we are at the end, remove the chunk
			//
			match chunk.get_ref().as_ref().len() as u64
			{
				x if x == chunk.position() => self.state = None,
				x if x  < chunk.position() => debug_assert!( false, "Attempted to consume more than available bytes" ),
				_                          => {}
			}
		}

		else
		{
			debug_assert!( false, "Attempted to consume from IntoAsyncRead without chunk" );
		}
	}
}

