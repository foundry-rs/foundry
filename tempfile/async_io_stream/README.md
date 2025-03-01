# async_io_stream

[![standard-readme compliant](https://img.shields.io/badge/readme%20style-standard-brightgreen.svg?style=flat-square)](https://github.com/RichardLitt/standard-readme)
[![Build Status](https://github.com/najamelan/async_io_stream/workflows/ci/badge.svg?branch=master)](https://github.com/najamelan/async_io_stream/actions)
[![Docs](https://docs.rs/async_io_stream/badge.svg)](https://docs.rs/async_io_stream)
[![crates.io](https://img.shields.io/crates/v/async_io_stream.svg)](https://crates.io/crates/async_io_stream)


> IntoAsyncRead on steroids

Provides a similar functionality as [`futures-util::IntoAsyncRead`](https://docs.rs/futures/0.3.4/futures/stream/trait.TryStreamExt.html#method.into_async_read). This crate handles both AsyncRead and AsyncWrite for an underlying type
that implements `Stream` and `Sink`. The stream needs to be a `TryStream` over `I: AsRef<u8>` and `std::io::Error`. The `Sink`
must be over `I: From< Vec<u8> >` with the same error.

The main other difference is that we will always try to use the complete buffer(s) provided by clients. That is for `poll_read`,
if more items are available on the `Stream`, we try to fill the entire buffer by using several messages. Implementations are
provided for vectored io in order to use all buffers maximally, compared to the default implementation which would only take
into account the first buffer.

For the `Sink` all data passed in is made into one item of the `Sink`.

[`AsyncBufRead`](https://docs.rs/futures/0.3.4/futures/io/trait.AsyncBufRead.html) is also implemented, which can be used to
avoid a copy of the data when reading.

Care is taken when polling the underlying `Stream` several times, to send a dummy waker so the underlying `Stream` doesn't try to wake up the task when we didn't return `Poll::Pending`. This is, if we already have data to return, we can't return `Poll::Pending`. If the underlying `Stream` returns an error, we will buffer it for the next poll.


## Table of Contents

- [Install](#install)
   - [Upgrade](#upgrade)
   - [Dependencies](#dependencies)
   - [Security](#security)
- [Usage](#usage)
   - [Basic Example](#basic-example)
   - [API](#api)
- [Contributing](#contributing)
   - [Code of Conduct](#code-of-conduct)
- [License](#license)


## Install
With [cargo add](https://github.com/killercup/cargo-edit):
`cargo add async_io_stream`

With [cargo yaml](https://gitlab.com/storedbox/cargo-yaml):
```yaml
dependencies:

   async_io_stream: ^0.3
```

With Cargo.toml
```toml
[dependencies]

    async_io_stream = "0.3"
```

### Upgrade

Please check out the [changelog](https://github.com/najamelan/async_io_stream/blob/master/CHANGELOG.md) when upgrading.


### Dependencies

This crate has few dependencies. Cargo will automatically handle it's dependencies for you.

Optionally with the `map_pharos` feature, the `Observable` trait is re-implemented and forwarded to the inner type.
This allows out of band error handling, as `AsyncRead`/`AsyncWrite` can only return `std::io::Error` and codecs will usually
stop processing the transport as soon as any error is returned. This allows notifying clients of non-fatal errors or events.

When the `tokio_io` feature is enabled, implementation for the traits `AsyncRead`/`AsyncWrite` from tokio are provided.


### Security

This crate uses `#![ forbid(unsafe_code) ]`. There is no maximum size protection for the buffers. The crate has not been
fuzz tested as we never interprete any of the data that passes through.


## Usage

### Basic example

```rust
use
{
   async_io_stream :: { IoStream              } ,
   futures::io     :: { AsyncWrite, AsyncRead } ,
   futures         :: { Stream, Sink          } ,
   std             :: { io                    } ,
};

fn usage( transport: impl Stream< Item=Result<Vec<u8>, io::Error> > + Sink< Vec<u8>, Error=io::Error > + Unpin )

   -> impl AsyncRead + AsyncWrite + Unpin
{
	IoStream::new( transport )
}

```

## API

API documentation can be found on [docs.rs](https://docs.rs/async_io_stream).


## Contributing

Please check out the [contribution guidelines](https://github.com/najamelan/async_io_stream/blob/master/CONTRIBUTING.md).


### Testing


### Code of conduct

Any of the behaviors described in [point 4 "Unacceptable Behavior" of the Citizens Code of Conduct](https://github.com/stumpsyn/policies/blob/master/citizen_code_of_conduct.md#4-unacceptable-behavior) are not welcome here and might get you banned. If anyone including maintainers and moderators of the project fail to respect these/your limits, you are entitled to call them out.

## License

[Unlicence](https://unlicense.org/)

